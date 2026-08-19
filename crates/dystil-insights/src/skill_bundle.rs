//! Durable, headless generation of portable prompt and Agent Skill bundles.

use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use chrono::Utc;
use dystil_ai::{AiAutomationRequest, AiRuntime};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    fingerprint, InsightsError, Result, SkillBundleStage, SkillBundleStatus, SkillBundleView,
    SkillInstallReceipt, SkillInstallTarget,
};

pub const SKILL_BUNDLE_BUILDER_VERSION: &str = "dystil-skill-bundle-v5+reviewed-grounding";
pub const WORKFLOW_RECONSTRUCTION_VERSION: &str = "dystil-workflow-reconstruction-v1";
const PROFILE: &str = include_str!("../resources/skill_bundle_builder_prompt.md");
const RECONSTRUCTION_PROFILE: &str = include_str!("../resources/workflow_reconstruction_prompt.md");
const REVIEW_PROFILE: &str = include_str!("../resources/skill_bundle_review_prompt.md");
const CREATOR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/resources/skill-creator");
const MAX_FILES: usize = 128;
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_RECONSTRUCTION_BYTES: u64 = 512 * 1024;
const BINARY_ASSET_EXTENSIONS: &[&str] = &["gif", "ico", "jpeg", "jpg", "pdf", "png", "webp"];
const FORBIDDEN_OUTPUT_NAMES: &[&str] = &[
    "PLAN.md",
    "README.md",
    "CHANGELOG.md",
    "workflow-understanding.md",
    "open-questions.json",
    "eval",
    "evaluation",
    "eval-workspace",
];

#[derive(Debug, Clone, Serialize)]
struct BuildFingerprint<'a> {
    artifact_id: &'a str,
    artifact_version: i64,
    intent_md: &'a str,
    reconstruction_seed_md: &'a str,
    reconstruction_version: &'a str,
    builder: &'a str,
}

#[derive(Debug, Clone)]
struct PreparedBuildInput {
    artifact_version: i64,
    intent_markdown: String,
    reconstruction_seed_markdown: String,
    evidence_ids: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct CachedReconstruction {
    body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BundleReviewVerdict {
    Approved,
    Rewrite,
}

#[derive(Debug, Clone)]
struct BundleReview {
    verdict: BundleReviewVerdict,
    corrections: String,
}

#[derive(Debug, Clone)]
pub struct SkillBundlePaths {
    pub builds_root: PathBuf,
    pub bundles_root: PathBuf,
}

impl SkillBundlePaths {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            builds_root: data_dir.as_ref().join("skill-bundle-builds"),
            bundles_root: data_dir.as_ref().join("skill-bundles"),
        }
    }
}

fn status(value: &str) -> Result<SkillBundleStatus> {
    match value {
        "pending" => Ok(SkillBundleStatus::Pending),
        "running" => Ok(SkillBundleStatus::Running),
        "ready" => Ok(SkillBundleStatus::Ready),
        "failed" => Ok(SkillBundleStatus::Failed),
        "interrupted" => Ok(SkillBundleStatus::Interrupted),
        _ => Err(InsightsError::Invalid(format!(
            "unknown skill bundle status {value}"
        ))),
    }
}

/// Marks builds left in-flight by a previous Dystil process as retryable.
///
/// Provider automation is owned by the app process and cannot safely be resumed
/// after that process exits. The next build therefore receives a fresh job and
/// workspace instead of inheriting partial provider output.
pub async fn interrupt_abandoned_skill_bundle_builds(pool: &SqlitePool) -> Result<u64> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE artifact_bundle_jobs
         SET status='interrupted',stage='failed',error_code='app_closed',
             error_message='Dystil was closed before this skill finished building.',
             updated_at=?1,finished_at=?1
         WHERE status IN ('pending','running')",
    )
    .bind(now)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

fn stage(value: Option<&str>) -> Result<Option<SkillBundleStage>> {
    match value {
        None => Ok(None),
        Some("preparing") => Ok(Some(SkillBundleStage::Preparing)),
        Some("investigating") => Ok(Some(SkillBundleStage::Investigating)),
        Some("building") => Ok(Some(SkillBundleStage::Building)),
        Some("validating") => Ok(Some(SkillBundleStage::Validating)),
        Some("ready") => Ok(Some(SkillBundleStage::Ready)),
        Some("failed") => Ok(Some(SkillBundleStage::Failed)),
        Some(value) => Err(InsightsError::Invalid(format!(
            "unknown skill bundle stage {value}"
        ))),
    }
}

fn view(row: &sqlx::sqlite::SqliteRow) -> Result<SkillBundleView> {
    Ok(SkillBundleView {
        job_id: row.try_get("job_id").ok(),
        bundle_id: row.try_get("bundle_id").ok(),
        revision: row
            .try_get::<i64, _>("revision")
            .ok()
            .map(|value| value as u32),
        skill_name: row.try_get("skill_name").ok(),
        status: status(row.get("status"))?,
        stage: stage(
            row.try_get::<Option<String>, _>("stage")
                .ok()
                .flatten()
                .as_deref(),
        )?,
        provider: row.try_get("provider").ok(),
        model: row.try_get("model").ok(),
        error_message: row.try_get("error_message").ok(),
    })
}

pub async fn ready_artifact_skill_bundle(
    pool: &SqlitePool,
    artifact_id: &str,
) -> Result<SkillBundleView> {
    let row = sqlx::query(
        "SELECT j.job_id,j.status,j.stage,j.provider,j.model,j.error_message,b.bundle_id,b.revision,b.skill_name
         FROM artifact_bundle_jobs j LEFT JOIN artifact_bundles b ON b.job_id=j.job_id
         WHERE j.artifact_id=?1 ORDER BY j.created_at DESC LIMIT 1"
    ).bind(artifact_id).fetch_optional(pool).await?;
    row.map(|row| view(&row)).transpose()?.ok_or_else(|| {
        InsightsError::Invalid("no skill bundle has been built for this artifact".into())
    })
}

async fn build_markdown(pool: &SqlitePool, artifact_id: &str) -> Result<(i64, String)> {
    let artifact = sqlx::query(
        "SELECT a.current_version,a.source_finding_id,a.kind,a.title,v.body FROM artifacts a
         JOIN artifact_versions v ON v.artifact_id=a.artifact_id AND v.ordinal=a.current_version
         WHERE a.artifact_id=?1 AND a.status='active'",
    )
    .bind(artifact_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| InsightsError::Invalid("artifact is not active".into()))?;
    let version: i64 = artifact.get("current_version");
    let finding_id: Option<String> = artifact.get("source_finding_id");
    let mut output = format!("# Build a reusable prompt and Agent Skill\n\n## Approved finding\n\n- Artifact: `{artifact_id}`\n- Existing handoff type: `{}`\n\n### Existing reusable handoff\n\n{}\n", artifact.get::<String,_>("kind"), artifact.get::<String,_>("body"));
    if let Some(finding_id) = finding_id {
        let finding = sqlx::query("SELECT claim,why_worth_fixing,evidence_note,handoff_preview,occurrence_count,cadence FROM findings WHERE finding_id=?1")
            .bind(&finding_id).fetch_optional(pool).await?;
        if let Some(finding) = finding {
            output.push_str(&format!("\n- Finding: `{finding_id}`\n\n### What was found\n\n{}\n\n### Why it was worth fixing\n\n{}\n\n### Evidence note\n\n{}\n\n### Preview steps\n\n{}\n\n- Occurrences: {}\n- Cadence: {}\n", finding.get::<String,_>("claim"), finding.get::<String,_>("why_worth_fixing"), finding.get::<String,_>("evidence_note"), finding.get::<String,_>("handoff_preview"), finding.get::<i64,_>("occurrence_count"), finding.get::<String,_>("cadence")));
            let evidence = sqlx::query("SELECT e.evidence_id,e.occurred_at,e.app,e.url,e.excerpt FROM finding_evidence fe JOIN evidence e ON e.evidence_id=fe.evidence_id WHERE fe.finding_id=?1 AND e.policy_allowed=1 AND e.redaction_ready=1 AND e.deleted=0 AND e.sensitive=0 ORDER BY e.occurred_at,e.evidence_id")
                .bind(&finding_id).fetch_all(pool).await?;
            output.push_str("\n## Cited textual evidence\n");
            for evidence in evidence {
                output.push_str(&format!(
                    "\n### `{}`\n\n- When: {}\n- Application: {}\n- URL: {}\n- Text: {}\n",
                    evidence.get::<String, _>("evidence_id"),
                    evidence.get::<String, _>("occurred_at"),
                    evidence
                        .try_get::<Option<String>, _>("app")
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| "none".into()),
                    evidence
                        .try_get::<Option<String>, _>("url")
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| "none".into()),
                    evidence.get::<String, _>("excerpt")
                ));
            }
        }
    }
    output.push_str("\n## Required output\n\nProduce `output/prompt.md` and one portable skill under `output/skill/`.\n");
    Ok((version, output))
}

async fn prepare_build_input(pool: &SqlitePool, artifact_id: &str) -> Result<PreparedBuildInput> {
    let (artifact_version, intent_markdown) = build_markdown(pool, artifact_id).await?;
    let artifact = sqlx::query(
        "SELECT source_finding_id,source_request_id,title FROM artifacts WHERE artifact_id=?1 AND status='active'",
    )
    .bind(artifact_id)
    .fetch_one(pool)
    .await?;
    let finding_id: Option<String> = artifact.get("source_finding_id");
    let mut seed = format!(
        "# Workflow reconstruction seed\n\n- Artifact: `{artifact_id}`\n- Title: {}\n\nThis is an evidence anchor set, not the complete workflow. Investigate related retained textual activity.\n",
        artifact.get::<String, _>("title")
    );
    let mut evidence_ids = BTreeSet::new();
    if let Some(finding_id) = finding_id {
        let opportunity_id: String =
            sqlx::query_scalar("SELECT opportunity_id FROM findings WHERE finding_id=?1")
                .bind(&finding_id)
                .fetch_one(pool)
                .await?;
        seed.push_str(&format!("\n## Worth Fixing origin\n\n- Finding: `{finding_id}`\n- Opportunity: `{opportunity_id}`\n"));
        let occurrences = sqlx::query(
            "SELECT occurrence_id,started_at,ended_at,observation_ids_json,evidence_ids_json,proposal_json
             FROM occurrences WHERE opportunity_id=?1 ORDER BY started_at",
        )
        .bind(&opportunity_id)
        .fetch_all(pool)
        .await?;
        for row in occurrences {
            let observation_ids: Vec<String> =
                serde_json::from_str(row.get("observation_ids_json"))?;
            let occurrence_evidence: Vec<String> =
                serde_json::from_str(row.get("evidence_ids_json"))?;
            evidence_ids.extend(occurrence_evidence.iter().cloned());
            let proposal: serde_json::Value = serde_json::from_str(row.get("proposal_json"))?;
            let steps = proposal
                .get("steps")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join("; ");
            seed.push_str(&format!(
                "\n### Occurrence `{}`\n\n- Time: {} to {}\n- Observation IDs: {}\n- Evidence IDs: {}\n- Recorded steps: {}\n",
                row.get::<String, _>("occurrence_id"),
                row.get::<String, _>("started_at"),
                row.get::<String, _>("ended_at"),
                observation_ids.join(", "),
                occurrence_evidence.join(", "),
                if steps.is_empty() { "none" } else { &steps }
            ));
            for observation_id in observation_ids {
                if let Some(statement) = sqlx::query_scalar::<_, String>(
                    "SELECT statement FROM observations WHERE observation_id=?1",
                )
                .bind(&observation_id)
                .fetch_optional(pool)
                .await?
                {
                    seed.push_str(&format!("- Observation `{observation_id}`: {statement}\n"));
                }
            }
        }
        let cited = sqlx::query_scalar::<_, String>(
            "SELECT evidence_id FROM finding_evidence WHERE finding_id=?1",
        )
        .bind(&finding_id)
        .fetch_all(pool)
        .await?;
        evidence_ids.extend(cited);
    } else {
        let request_id: Option<String> = artifact.get("source_request_id");
        seed.push_str(
            "\n## Ask or direct origin\n\nUse the approved intent as the starting search query.\n",
        );
        if let Some(request_id) = request_id {
            seed.push_str(&format!("- Source request: `{request_id}`\n"));
        }
        if let Some(row) = sqlx::query(
            "SELECT session_id,locked_understanding_json,understanding_json,presentation_json
             FROM ask_sessions WHERE artifact_kept_id=?1",
        )
        .bind(artifact_id)
        .fetch_optional(pool)
        .await?
        {
            let session_id: String = row.get("session_id");
            seed.push_str(&format!(
                "\n### Ask-for-Fix context `{}`\n\n- Understanding: {}\n- Presentation: {}\n",
                session_id,
                row.try_get::<Option<String>, _>("locked_understanding_json")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| row.get::<String, _>("understanding_json")),
                row.try_get::<Option<String>, _>("presentation_json")
                    .ok()
                    .flatten()
                    .unwrap_or_default()
            ));
            let messages = sqlx::query(
                "SELECT role,text FROM ask_messages WHERE session_id=?1 ORDER BY ordinal LIMIT 32",
            )
            .bind(&session_id)
            .fetch_all(pool)
            .await?;
            if !messages.is_empty() {
                seed.push_str("\n### Relevant Ask conversation\n");
                for message in messages {
                    seed.push_str(&format!(
                        "\n- {}: {}\n",
                        message.get::<String, _>("role"),
                        message.get::<String, _>("text")
                    ));
                }
            }
            if let Some(report) = sqlx::query(
                "SELECT report_json,memo FROM ask_retrieval_reports
                 WHERE session_id=?1 AND status='ready' ORDER BY updated_at DESC LIMIT 1",
            )
            .bind(&session_id)
            .fetch_optional(pool)
            .await?
            {
                let report_json: String = report.get("report_json");
                let grounding = serde_json::from_str::<serde_json::Value>(&report_json)
                    .ok()
                    .and_then(|value| value.get("groundingIds").cloned())
                    .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
                    .unwrap_or_default();
                evidence_ids.extend(grounding.iter().cloned());
                seed.push_str(&format!(
                    "\n### Accepted retrieval memo\n\n{}\n\n- Grounding IDs: {}\n",
                    report.get::<String, _>("memo"),
                    grounding.join(", ")
                ));
            }
        }
    }
    seed.push_str("\n## Admitted evidence details\n");
    for evidence_id in &evidence_ids {
        if let Some(row) = sqlx::query(
            "SELECT occurred_at,app,window,url,excerpt FROM evidence
             WHERE evidence_id=?1 AND policy_allowed=1 AND redaction_ready=1 AND deleted=0 AND sensitive=0",
        )
        .bind(evidence_id)
        .fetch_optional(pool)
        .await?
        {
            seed.push_str(&format!(
                "\n### `{evidence_id}`\n\n- When: {}\n- Application: {}\n- Window: {}\n- URL: {}\n- Text: {}\n",
                row.get::<String, _>("occurred_at"),
                row.try_get::<Option<String>, _>("app").ok().flatten().unwrap_or_else(|| "none".into()),
                row.try_get::<Option<String>, _>("window").ok().flatten().unwrap_or_else(|| "none".into()),
                row.try_get::<Option<String>, _>("url").ok().flatten().unwrap_or_else(|| "none".into()),
                row.get::<String, _>("excerpt")
            ));
        }
    }
    Ok(PreparedBuildInput {
        artifact_version,
        intent_markdown,
        reconstruction_seed_markdown: seed,
        evidence_ids,
    })
}

fn reconstruction_evidence_ids(markdown: &str) -> BTreeSet<String> {
    markdown
        .split(|character: char| {
            !(character.is_ascii_alphanumeric() || matches!(character, ':' | '_' | '-'))
        })
        .map(|token| token.trim_end_matches(':'))
        .filter(|token| {
            token.contains(':')
                && (token.starts_with("local-capture:")
                    || token.starts_with("frame:")
                    || token.starts_with("event:"))
        })
        .map(str::to_owned)
        .collect()
}

async fn validate_reconstruction(
    pool: &SqlitePool,
    path: &Path,
    _anchor_evidence_ids: &BTreeSet<String>,
) -> Result<(String, BTreeSet<String>, BTreeSet<String>)> {
    let body = fs::read_to_string(path).map_err(|_| {
        InsightsError::Invalid("workflow reconstruction is missing input/WORKFLOW.md".into())
    })?;
    if body.len() as u64 > MAX_RECONSTRUCTION_BYTES || body.trim().len() < 300 {
        return Err(InsightsError::Invalid(
            "workflow reconstruction is empty or exceeds its size limit".into(),
        ));
    }
    for section in [
        "# Workflow reconstruction",
        "## Task outcome and boundaries",
        "## Trigger and starting state",
        "## Inputs and source discovery",
        "## Systems, surfaces, and access",
        "## Observed end-to-end workflow",
        "## Decisions, variants, and exceptions",
        "## Outputs, destinations, and naming",
        "## Validation and completion signals",
        "## Runtime execution strategy",
        "## Evidence map",
        "## Unknowns and runtime discovery",
    ] {
        if !body.contains(section) {
            return Err(InsightsError::Invalid(format!(
                "workflow reconstruction is missing {section}"
            )));
        }
    }
    let ids = reconstruction_evidence_ids(&body);
    // Evidence references are grounding aids for the investigator, not a
    // machine-readable protocol. Deep retrieval can find raw source IDs that
    // are absent from the compact insights projection, and models may use
    // simple labels such as E1/E2 instead. Persist recognizable IDs when they
    // are present, but do not reject useful workflow understanding over their
    // format or projection membership.
    let _ = pool;
    Ok((body.clone(), ids, urls_in(&body)))
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn safe_relative(path: &Path) -> bool {
    path.components()
        .all(|part| matches!(part, Component::Normal(_)))
}

fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[derive(Serialize)]
struct ManifestFile {
    path: String,
    sha256: String,
    bytes: u64,
}

fn urls_in(text: &str) -> BTreeSet<String> {
    text.split_whitespace()
        .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
        .map(|value| {
            value
                .trim_matches(|character: char| matches!(character, '`' | ')' | ']' | ',' | '.'))
                .to_string()
        })
        .collect()
}

#[cfg(test)]
fn validate_output(output: &Path) -> Result<(String, String, String, String)> {
    validate_output_with_urls(output, &BTreeSet::new())
}

fn validate_output_with_urls(
    output: &Path,
    allowed_urls: &BTreeSet<String>,
) -> Result<(String, String, String, String)> {
    prune_empty_optional_directories(output)?;
    validate_output_layout(output)?;
    let prompt = output.join("prompt.md");
    if fs::read_to_string(&prompt)
        .map_err(|_| {
            InsightsError::Invalid("bundle is missing a non-empty output/prompt.md".into())
        })?
        .trim()
        .is_empty()
    {
        return Err(InsightsError::Invalid("bundle prompt.md is empty".into()));
    }
    let skill_root = output.join("skill");
    let skill_entries = fs::read_dir(&skill_root)
        .map_err(|_| InsightsError::Invalid("bundle is missing output/skill".into()))?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();
    if skill_entries.len() != 1
        || !skill_entries[0]
            .file_type()
            .map(|kind| kind.is_dir() && !kind.is_symlink())
            .unwrap_or(false)
    {
        return Err(InsightsError::Invalid(
            "output/skill must contain exactly one non-symlink skill directory".into(),
        ));
    }
    let skill = &skill_entries[0];
    let name = skill.file_name().to_string_lossy().to_string();
    if !valid_skill_name(&name) {
        return Err(InsightsError::Invalid(
            "skill directory name must be lowercase kebab-case and at most 64 characters".into(),
        ));
    }
    let skill_md = skill.path().join("SKILL.md");
    let text = fs::read_to_string(&skill_md)
        .map_err(|_| InsightsError::Invalid("skill is missing SKILL.md".into()))?;
    let mut front = text
        .strip_prefix("---\n")
        .and_then(|value| value.split_once("\n---"))
        .map(|(value, _)| value.lines());
    let name_line = front
        .as_mut()
        .and_then(|lines| lines.find(|line| line.starts_with("name:")).map(str::trim));
    let description = text
        .lines()
        .skip_while(|line| *line != "---")
        .skip(1)
        .take_while(|line| *line != "---")
        .find(|line| line.starts_with("description:"))
        .map(str::trim);
    if name_line != Some(format!("name: {name}").as_str())
        || description.is_none_or(|line| line == "description:")
    {
        return Err(InsightsError::Invalid(
            "SKILL.md frontmatter must contain matching name and non-empty description".into(),
        ));
    }
    let workflow = skill.path().join("references/workflow.md");
    if fs::read_to_string(&workflow)
        .map_err(|_| InsightsError::Invalid("skill is missing references/workflow.md".into()))?
        .trim()
        .is_empty()
        || !text.contains("references/workflow.md")
    {
        return Err(InsightsError::Invalid(
            "SKILL.md must reference a non-empty references/workflow.md".into(),
        ));
    }
    let mut files = Vec::new();
    collect_files(output, output, &mut files)?;
    for file in &files {
        if !is_binary_asset(Path::new(&file.path)) {
            let content = fs::read_to_string(output.join(&file.path))
                .map_err(|_| InsightsError::Invalid("bundle text file must be UTF-8".into()))?;
            if content.contains("local-capture:") || content.contains("evidence_id") {
                return Err(InsightsError::Invalid(
                    "portable bundle may not include Dystil evidence IDs".into(),
                ));
            }
            for url in urls_in(&content) {
                if !allowed_urls.contains(&url) {
                    return Err(InsightsError::Invalid(format!(
                        "portable bundle contains a URL not supported by the workflow reconstruction: {url}"
                    )));
                }
            }
        }
    }
    if files.len() > MAX_FILES
        || files.iter().any(|file| file.bytes > MAX_FILE_BYTES)
        || files
            .iter()
            .map(|file: &ManifestFile| file.bytes)
            .sum::<u64>()
            > MAX_BYTES
    {
        return Err(InsightsError::Invalid(
            "bundle exceeds file-count or size limit".into(),
        ));
    }
    validate_skill_references(&skill.path(), &text)?;
    validate_openai_metadata(&skill.path())?;
    let manifest = serde_json::to_string(&files)?;
    // The manifest binds names, ordering, sizes, and per-file digests. Include
    // the actual bytes as well so the receipt is explicitly over the canonical
    // manifest *and* content, not only a serialization of the manifest.
    let mut receipt = Sha256::new();
    receipt.update(manifest.as_bytes());
    for file in &files {
        receipt.update(file.path.as_bytes());
        receipt.update(
            fs::read(output.join(&file.path))
                .map_err(|error| InsightsError::Invalid(error.to_string()))?,
        );
    }
    let checksum = hex::encode(receipt.finalize());
    let skill_path = format!("skill/{name}");
    Ok((name, manifest, checksum, skill_path))
}

fn markdown_section<'a>(body: &'a str, heading: &str) -> Option<&'a str> {
    let start = body.find(heading)? + heading.len();
    let remaining = &body[start..];
    let end = remaining.find("\n## ").unwrap_or(remaining.len());
    Some(remaining[..end].trim())
}

fn validate_bundle_review(path: &Path) -> Result<BundleReview> {
    let body = fs::read_to_string(path).map_err(|_| {
        InsightsError::Invalid("bundle reviewer did not write BUNDLE_REVIEW.md".into())
    })?;
    if !body.starts_with("# Bundle review\n") {
        return Err(InsightsError::Invalid(
            "bundle review must start with its required heading".into(),
        ));
    }
    let verdict = markdown_section(&body, "## Verdict")
        .ok_or_else(|| InsightsError::Invalid("bundle review is missing Verdict".into()))?
        .to_ascii_lowercase();
    let corrections = markdown_section(&body, "## Required corrections")
        .ok_or_else(|| {
            InsightsError::Invalid("bundle review is missing Required corrections".into())
        })?
        .to_string();
    let verdict = match verdict.as_str() {
        "approved" => BundleReviewVerdict::Approved,
        "rewrite" if !corrections.is_empty() && corrections != "None" => {
            BundleReviewVerdict::Rewrite
        }
        "rewrite" => {
            return Err(InsightsError::Invalid(
                "rewrite bundle review must include concrete corrections".into(),
            ));
        }
        _ => {
            return Err(InsightsError::Invalid(
                "bundle review verdict must be approved or rewrite".into(),
            ));
        }
    };
    Ok(BundleReview {
        verdict,
        corrections,
    })
}

/// Keep the persisted artifact deliberately small and portable. The builder has
/// a whole workspace for its authoring material, but only the common Agent Skill
/// layout is permitted to cross the validation boundary into a bundle revision.
fn validate_output_layout(output: &Path) -> Result<()> {
    let root_entries =
        fs::read_dir(output).map_err(|error| InsightsError::Invalid(error.to_string()))?;
    for entry in root_entries {
        let entry = entry.map_err(|error| InsightsError::Invalid(error.to_string()))?;
        let kind = entry
            .file_type()
            .map_err(|error| InsightsError::Invalid(error.to_string()))?;
        let name = entry.file_name();
        match name.to_string_lossy().as_ref() {
            "prompt.md" if kind.is_file() && !kind.is_symlink() => {}
            "skill" if kind.is_dir() && !kind.is_symlink() => {}
            _ => {
                return Err(InsightsError::Invalid(
                    "bundle output may contain only prompt.md and skill/".into(),
                ));
            }
        }
    }

    let skill_root = output.join("skill");
    let entries = fs::read_dir(&skill_root)
        .map_err(|_| InsightsError::Invalid("bundle is missing output/skill".into()))?;
    for entry in entries {
        let entry = entry.map_err(|error| InsightsError::Invalid(error.to_string()))?;
        let kind = entry
            .file_type()
            .map_err(|error| InsightsError::Invalid(error.to_string()))?;
        if !kind.is_dir() || kind.is_symlink() {
            return Err(InsightsError::Invalid(
                "output/skill may contain only one non-symlink skill directory".into(),
            ));
        }
        validate_skill_layout(&entry.path())?;
    }
    Ok(())
}

fn validate_skill_layout(skill: &Path) -> Result<()> {
    for entry in fs::read_dir(skill).map_err(|error| InsightsError::Invalid(error.to_string()))? {
        let entry = entry.map_err(|error| InsightsError::Invalid(error.to_string()))?;
        let kind = entry
            .file_type()
            .map_err(|error| InsightsError::Invalid(error.to_string()))?;
        if kind.is_symlink() {
            return Err(InsightsError::Invalid(
                "bundle may not contain symlinks".into(),
            ));
        }
        let name = entry.file_name();
        match name.to_string_lossy().as_ref() {
            "SKILL.md" if kind.is_file() => {}
            "references" | "scripts" | "assets" if kind.is_dir() => {}
            "agents" if kind.is_dir() => validate_agents_directory(&entry.path())?,
            _ => {
                return Err(InsightsError::Invalid(
                    "skill may contain only SKILL.md and optional references/, scripts/, assets/, or agents/openai.yaml"
                        .into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_agents_directory(agents: &Path) -> Result<()> {
    for entry in fs::read_dir(agents).map_err(|error| InsightsError::Invalid(error.to_string()))? {
        let entry = entry.map_err(|error| InsightsError::Invalid(error.to_string()))?;
        let kind = entry
            .file_type()
            .map_err(|error| InsightsError::Invalid(error.to_string()))?;
        if entry.file_name() != "openai.yaml" || !kind.is_file() || kind.is_symlink() {
            return Err(InsightsError::Invalid(
                "agents/ may contain only a non-symlink openai.yaml file".into(),
            ));
        }
    }
    Ok(())
}

fn prune_empty_optional_directories(output: &Path) -> Result<()> {
    for directory in ["references", "scripts", "assets", "agents"] {
        prune_named_directories(output, directory)?;
    }
    Ok(())
}

fn prune_named_directories(root: &Path, name: &str) -> Result<()> {
    for entry in fs::read_dir(root).map_err(|error| InsightsError::Invalid(error.to_string()))? {
        let entry = entry.map_err(|error| InsightsError::Invalid(error.to_string()))?;
        let kind = entry
            .file_type()
            .map_err(|error| InsightsError::Invalid(error.to_string()))?;
        if kind.is_symlink() {
            return Err(InsightsError::Invalid(
                "bundle may not contain symlinks".into(),
            ));
        }
        if kind.is_dir() {
            prune_named_directories(&entry.path(), name)?;
            if entry.file_name() == name
                && fs::read_dir(entry.path())
                    .map_err(|error| InsightsError::Invalid(error.to_string()))?
                    .next()
                    .is_none()
            {
                fs::remove_dir(entry.path())
                    .map_err(|error| InsightsError::Invalid(error.to_string()))?;
            }
        }
    }
    Ok(())
}

fn is_binary_asset(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| BINARY_ASSET_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn validate_skill_references(skill: &Path, text: &str) -> Result<()> {
    let mut references = BTreeSet::new();
    let mut rest = text;
    while let Some((_, after_open)) = rest.split_once("](") {
        let Some((target, after_close)) = after_open.split_once(')') else {
            break;
        };
        rest = after_close;
        let target = target
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches('<');
        if target.is_empty()
            || target.starts_with('#')
            || target.contains("://")
            || target.starts_with("mailto:")
        {
            continue;
        }
        references.insert(target.trim_end_matches('>').to_string());
    }
    for line in text.lines() {
        for segment in line.split('`').skip(1).step_by(2) {
            if segment.starts_with("references/")
                || segment.starts_with("scripts/")
                || segment.starts_with("assets/")
                || segment.starts_with("agents/")
            {
                references.insert(segment.to_string());
            }
        }
    }
    for reference in references {
        let path = Path::new(&reference);
        if path.is_absolute() || !safe_relative(path) || !skill.join(path).is_file() {
            return Err(InsightsError::Invalid(format!(
                "SKILL.md references a missing or unsafe local file: {reference}"
            )));
        }
    }
    for banned in [
        "builder/skill-creator",
        "skill-bundle-builds",
        "worth-fixing.sqlite",
        "evidence_id",
    ] {
        if text.contains(banned) {
            return Err(InsightsError::Invalid(format!(
                "SKILL.md must not require temporary Dystil build data ({banned})"
            )));
        }
    }
    for invocation in ["$skill-", "/skill-"] {
        if text.contains(invocation) {
            return Err(InsightsError::Invalid(
                "SKILL.md contains provider-specific invocation syntax".into(),
            ));
        }
    }
    if text.contains("`!") || text.lines().any(|line| line.trim_start().starts_with('!')) {
        return Err(InsightsError::Invalid(
            "SKILL.md contains provider-specific invocation syntax".into(),
        ));
    }
    Ok(())
}

fn validate_openai_metadata(skill: &Path) -> Result<()> {
    let path = skill.join("agents/openai.yaml");
    if !path.exists() {
        return Ok(());
    }
    let yaml: serde_yaml::Value = serde_yaml::from_str(
        &fs::read_to_string(&path)
            .map_err(|_| InsightsError::Invalid("agents/openai.yaml must be UTF-8".into()))?,
    )
    .map_err(|error| {
        InsightsError::Invalid(format!("agents/openai.yaml is invalid YAML: {error}"))
    })?;
    let serialized = serde_yaml::to_string(&yaml).map_err(|error| {
        InsightsError::Invalid(format!("agents/openai.yaml cannot be read: {error}"))
    })?;
    for token in serialized.split(|character: char| {
        character.is_whitespace() || matches!(character, ':' | ',' | '[' | ']' | '\"' | '\'')
    }) {
        if token.starts_with("references/")
            || token.starts_with("scripts/")
            || token.starts_with("assets/")
        {
            let asset = Path::new(token);
            if !safe_relative(asset) || !skill.join(asset).is_file() {
                return Err(InsightsError::Invalid(format!(
                    "agents/openai.yaml references a missing local asset: {token}"
                )));
            }
        }
    }
    Ok(())
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<ManifestFile>) -> Result<()> {
    for entry in fs::read_dir(current).map_err(|error| InsightsError::Invalid(error.to_string()))? {
        let entry = entry.map_err(|error| InsightsError::Invalid(error.to_string()))?;
        let kind = entry
            .file_type()
            .map_err(|error| InsightsError::Invalid(error.to_string()))?;
        if kind.is_symlink() {
            return Err(InsightsError::Invalid(
                "bundle may not contain symlinks".into(),
            ));
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| InsightsError::Invalid("invalid output path".into()))?
            .to_path_buf();
        if !safe_relative(&relative) {
            return Err(InsightsError::Invalid(
                "bundle contains unsafe output path".into(),
            ));
        }
        if kind.is_dir() {
            if FORBIDDEN_OUTPUT_NAMES.contains(&entry.file_name().to_string_lossy().as_ref()) {
                return Err(InsightsError::Invalid(
                    "bundle contains a forbidden generated report".into(),
                ));
            }
            collect_files(root, &entry.path(), files)?;
        } else {
            let bytes = fs::read(entry.path())
                .map_err(|error| InsightsError::Invalid(error.to_string()))?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if FORBIDDEN_OUTPUT_NAMES.contains(&name.as_ref()) {
                return Err(InsightsError::Invalid(
                    "bundle contains a forbidden generated report".into(),
                ));
            }
            if !is_binary_asset(&entry.path()) && std::str::from_utf8(&bytes).is_err() {
                return Err(InsightsError::Invalid(format!(
                    "bundle text file is not UTF-8: {}",
                    relative.display()
                )));
            }
            files.push(ManifestFile {
                path: relative.to_string_lossy().into_owned(),
                sha256: hex::encode(Sha256::digest(&bytes)),
                bytes: bytes.len() as u64,
            });
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(())
}

/// A durable job already recorded in SQLite but not yet handed to a provider.
/// It is intentionally opaque outside this module: callers can start it and
/// later run it, but cannot alter the approved build input.
pub struct PendingSkillBundleBuild {
    job_id: String,
    artifact_id: String,
    artifact_version: i64,
    intent_markdown: String,
    reconstruction_seed_markdown: String,
    reconstruction_evidence_ids: BTreeSet<String>,
    cached_reconstruction: Option<CachedReconstruction>,
    input_fingerprint: String,
    working_directory: PathBuf,
}

pub async fn start_skill_bundle_build<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    artifact_id: &str,
    paths: &SkillBundlePaths,
) -> Result<(SkillBundleView, Option<PendingSkillBundleBuild>)> {
    let input = prepare_build_input(pool, artifact_id).await?;
    let input_fingerprint = fingerprint(&BuildFingerprint {
        artifact_id,
        artifact_version: input.artifact_version,
        intent_md: &input.intent_markdown,
        reconstruction_seed_md: &input.reconstruction_seed_markdown,
        reconstruction_version: WORKFLOW_RECONSTRUCTION_VERSION,
        builder: SKILL_BUNDLE_BUILDER_VERSION,
    })?;
    if let Some(row) = sqlx::query("SELECT j.job_id,j.status,j.stage,j.provider,j.model,j.error_message,b.bundle_id,b.revision,b.skill_name FROM artifact_bundle_jobs j LEFT JOIN artifact_bundles b ON b.job_id=j.job_id WHERE j.artifact_id=?1 AND j.input_fingerprint=?2 AND j.status='ready'").bind(artifact_id).bind(&input_fingerprint).fetch_optional(pool).await? {
        return Ok((view(&row)?, None));
    }
    let job_id = format!("sbj_{}", Uuid::new_v4().simple());
    let now = Utc::now().to_rfc3339();
    let working = paths.builds_root.join(&job_id);
    let inserted = sqlx::query("INSERT OR IGNORE INTO artifact_bundle_jobs(job_id,artifact_id,artifact_version,input_fingerprint,builder_version,status,stage,working_directory,provider,model,attempts,created_at,updated_at,started_at) VALUES(?1,?2,?3,?4,?5,'running','preparing',?6,?7,?8,1,?9,?9,?9)")
        .bind(&job_id).bind(artifact_id).bind(input.artifact_version).bind(&input_fingerprint).bind(SKILL_BUNDLE_BUILDER_VERSION).bind(working.to_string_lossy().to_string()).bind(&runtime.descriptor().provider_label).bind(&runtime.descriptor().model).bind(&now).execute(pool).await?;
    if inserted.rows_affected() == 0 {
        let row = sqlx::query("SELECT j.job_id,j.status,j.stage,j.provider,j.model,j.error_message,b.bundle_id,b.revision,b.skill_name FROM artifact_bundle_jobs j LEFT JOIN artifact_bundles b ON b.job_id=j.job_id WHERE j.artifact_id=?1 AND j.input_fingerprint=?2 ORDER BY j.created_at DESC LIMIT 1")
            .bind(artifact_id).bind(&input_fingerprint).fetch_one(pool).await?;
        return Ok((view(&row)?, None));
    }
    let view = SkillBundleView {
        job_id: Some(job_id.clone()),
        bundle_id: None,
        revision: None,
        skill_name: None,
        status: SkillBundleStatus::Running,
        stage: Some(SkillBundleStage::Preparing),
        provider: Some(runtime.descriptor().provider_label.clone()),
        model: Some(runtime.descriptor().model.clone()),
        error_message: None,
    };
    let cached_reconstruction = sqlx::query(
        "SELECT body FROM artifact_workflow_reconstructions
         WHERE artifact_id=?1 AND artifact_version=?2 AND input_fingerprint=?3
           AND reconstruction_version=?4
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(artifact_id)
    .bind(input.artifact_version)
    .bind(&input_fingerprint)
    .bind(WORKFLOW_RECONSTRUCTION_VERSION)
    .fetch_optional(pool)
    .await?
    .map(|row| CachedReconstruction {
        body: row.get("body"),
    });
    Ok((
        view,
        Some(PendingSkillBundleBuild {
            job_id,
            artifact_id: artifact_id.into(),
            artifact_version: input.artifact_version,
            intent_markdown: input.intent_markdown,
            reconstruction_seed_markdown: input.reconstruction_seed_markdown,
            reconstruction_evidence_ids: input.evidence_ids,
            cached_reconstruction,
            input_fingerprint,
            working_directory: working,
        }),
    ))
}

pub async fn run_skill_bundle_build<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    build: PendingSkillBundleBuild,
    paths: &SkillBundlePaths,
) -> Result<SkillBundleView> {
    let PendingSkillBundleBuild {
        job_id,
        artifact_id,
        artifact_version,
        intent_markdown,
        reconstruction_seed_markdown,
        reconstruction_evidence_ids,
        cached_reconstruction,
        input_fingerprint,
        working_directory: working,
    } = build;
    let outcome = async {
        fs::create_dir_all(working.join("input")).map_err(|error| InsightsError::Invalid(error.to_string()))?;
        fs::write(working.join("input/INTENT.md"), &intent_markdown).map_err(|error| InsightsError::Invalid(error.to_string()))?;
        fs::write(working.join("input/RECONSTRUCTION_SEED.md"), &reconstruction_seed_markdown).map_err(|error| InsightsError::Invalid(error.to_string()))?;
        fs::create_dir_all(working.join("output")).map_err(|error| InsightsError::Invalid(error.to_string()))?;
        let reconstruction_path = working.join("input/WORKFLOW.md");
        let (reconstruction_run, workflow, cited_ids, allowed_urls) = if let Some(cached) = cached_reconstruction {
            fs::write(&reconstruction_path, cached.body)
                .map_err(|error| InsightsError::Invalid(error.to_string()))?;
            let (workflow, cited_ids, allowed_urls) = validate_reconstruction(
                pool,
                &reconstruction_path,
                &reconstruction_evidence_ids,
            )
            .await?;
            (
                dystil_ai::AiAutomationRun {
                    runtime: runtime.descriptor().kind.clone(),
                    runtime_version: None,
                    elapsed_ms: 0,
                    output: "reused validated workflow reconstruction".into(),
                },
                workflow,
                cited_ids,
                allowed_urls,
            )
        } else {
            update_stage(pool, &job_id, "investigating").await?;
            let first_reconstruction_run = run_phase(runtime, RECONSTRUCTION_PROFILE, working.clone()).await?;
            match validate_reconstruction(
                pool,
                &reconstruction_path,
                &reconstruction_evidence_ids,
            )
            .await
            {
                Ok((workflow, cited_ids, allowed_urls)) => {
                    (first_reconstruction_run, workflow, cited_ids, allowed_urls)
                }
                Err(error) => {
                let repair_prompt = format!(
                    "{RECONSTRUCTION_PROFILE}\n\nYour first WORKFLOW.md failed Dystil's structural validation: {}\n\nRepair only input/WORKFLOW.md now. Keep supported facts and add every required section. Evidence labels are best-effort grounding only; do not spend this repair trying to match Dystil IDs. Do not invent literal URLs. Do not create any other file.",
                    error.to_string().chars().take(1_000).collect::<String>()
                );
                let repair_run = run_phase(runtime, &repair_prompt, working.clone()).await?;
                let (workflow, cited_ids, allowed_urls) = validate_reconstruction(
                    pool,
                    &reconstruction_path,
                    &reconstruction_evidence_ids,
                )
                .await?;
                let combined_run = dystil_ai::AiAutomationRun {
                    runtime: repair_run.runtime,
                    runtime_version: repair_run.runtime_version.or(first_reconstruction_run.runtime_version),
                    elapsed_ms: first_reconstruction_run.elapsed_ms + repair_run.elapsed_ms,
                    output: repair_run.output,
                };
                    (combined_run, workflow, cited_ids, allowed_urls)
                }
            }
        };
        let created = Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO artifact_workflow_reconstructions(reconstruction_id,bundle_job_id,artifact_id,artifact_version,input_fingerprint,body,evidence_ids_json,reconstruction_version,provider,model,runtime_version,elapsed_ms,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)")
            .bind(format!("swr_{}", Uuid::new_v4().simple())).bind(&job_id).bind(&artifact_id).bind(artifact_version).bind(&input_fingerprint).bind(&workflow).bind(serde_json::to_string(&cited_ids)?).bind(WORKFLOW_RECONSTRUCTION_VERSION).bind(&runtime.descriptor().provider_label).bind(&runtime.descriptor().model).bind(&reconstruction_run.runtime_version).bind(reconstruction_run.elapsed_ms as i64).bind(&created).execute(pool).await?;
        copy_tree(Path::new(CREATOR), &working.join("builder/skill-creator")).map_err(|error| InsightsError::Invalid(error.to_string()))?;
        update_stage(pool, &job_id, "building").await?;
        let first_builder_run = run_phase(runtime, PROFILE, working.clone()).await?;
        update_stage(pool, &job_id, "validating").await?;
        let (mut run, mut skill_name, mut manifest, mut checksum, mut skill_path) =
            match validate_output_with_urls(&working.join("output"), &allowed_urls) {
                Ok((skill_name, manifest, checksum, skill_path)) => (
                    first_builder_run,
                    skill_name,
                    manifest,
                    checksum,
                    skill_path,
                ),
                Err(error) => {
                    // A repair must start from a blank portable bundle. Providers otherwise
                    // tend to make a cosmetic edit to an invalid draft and preserve invented
                    // workflow advice that is no longer supported by the reconstruction.
                    let output = working.join("output");
                    if output.exists() {
                        fs::remove_dir_all(&output)
                            .map_err(|error| InsightsError::Invalid(error.to_string()))?;
                    }
                    fs::create_dir_all(&output)
                        .map_err(|error| InsightsError::Invalid(error.to_string()))?;
                    let repair_prompt = format!(
                        "The previous bundle was deleted. Build a new portable bundle from input/WORKFLOW.md now: output/prompt.md, output/skill/<skill-name>/SKILL.md, and output/skill/<skill-name>/references/workflow.md. Do not create output/manifest.json: Dystil stores manifest metadata itself. Do not read, restore, or imitate the deleted draft.\n\nUse only a conservative transformation of input/WORKFLOW.md: retain literal observed systems, names, routes, documents, steps, decisions, validation, completion boundaries, and explicitly stated runtime discovery. Do not add operational behavior, values, or workflow steps unless input/WORKFLOW.md supports them. When a detail is unknown, state the precise runtime-discovery check instead of choosing it. Never ask for a value merely because an imagined guide needs one; ask only for a precisely identified missing connection, file, folder, or current view.\n\nThe validation error was: {}\n\nGenerate no files outside output/. Ensure references/workflow.md exists and SKILL.md links to it.",
                        error.to_string().chars().take(1_000).collect::<String>()
                    );
                    let repair_run = run_phase(runtime, &repair_prompt, working.clone()).await?;
                    let (skill_name, manifest, checksum, skill_path) =
                        validate_output_with_urls(&working.join("output"), &allowed_urls)?;
                    let combined_run = dystil_ai::AiAutomationRun {
                        runtime: repair_run.runtime,
                        runtime_version: repair_run
                            .runtime_version
                            .or(first_builder_run.runtime_version),
                        elapsed_ms: first_builder_run.elapsed_ms + repair_run.elapsed_ms,
                        output: repair_run.output,
                    };
                    (combined_run, skill_name, manifest, checksum, skill_path)
                }
            };
        let review_path = working.join("input/BUNDLE_REVIEW.md");
        let first_review_run = run_phase(runtime, REVIEW_PROFILE, working.clone()).await?;
        let review = validate_bundle_review(&review_path)?;
        run.elapsed_ms += first_review_run.elapsed_ms;
        if review.verdict == BundleReviewVerdict::Rewrite {
            let output = working.join("output");
            fs::remove_dir_all(&output)
                .map_err(|error| InsightsError::Invalid(error.to_string()))?;
            fs::create_dir_all(&output)
                .map_err(|error| InsightsError::Invalid(error.to_string()))?;
            let repair_prompt = format!(
                "Read input/WORKFLOW.md and input/BUNDLE_REVIEW.md. Rebuild output/prompt.md and one portable skill under output/skill/<skill-name>/ from the workflow, applying every concrete correction in the review. Do not create output/manifest.json or files outside output/. Ensure SKILL.md has the required YAML frontmatter and links to references/workflow.md."
            );
            let repair_run = run_phase(runtime, &repair_prompt, working.clone()).await?;
            (skill_name, manifest, checksum, skill_path) =
                validate_output_with_urls(&working.join("output"), &allowed_urls)?;
            let verification_run = run_phase(runtime, REVIEW_PROFILE, working.clone()).await?;
            let verification = validate_bundle_review(&review_path)?;
            if verification.verdict != BundleReviewVerdict::Approved {
                return Err(InsightsError::Invalid(format!(
                    "bundle review still requires rewrite: {}",
                    verification.corrections
                )));
            }
            run.elapsed_ms += repair_run.elapsed_ms + verification_run.elapsed_ms;
        }
        let revision: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(revision),0)+1 FROM artifact_bundles WHERE artifact_id=?1").bind(&artifact_id).fetch_one(pool).await?;
        let final_dir = paths.bundles_root.join(&artifact_id).join(revision.to_string()); fs::create_dir_all(final_dir.parent().unwrap()).map_err(|error| InsightsError::Invalid(error.to_string()))?;
        fs::rename(working.join("output"), &final_dir).map_err(|error| InsightsError::Invalid(error.to_string()))?;
        let bundle_id = format!("sbb_{}", Uuid::new_v4().simple()); let done = Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO artifact_bundles(bundle_id,artifact_id,artifact_version,job_id,revision,skill_name,directory,prompt_path,skill_path,manifest_json,checksum,builder_version,provider,model,status,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'prompt.md',?8,?9,?10,?11,?12,?13,'ready',?14)")
            .bind(&bundle_id).bind(&artifact_id).bind(artifact_version).bind(&job_id).bind(revision).bind(&skill_name).bind(final_dir.to_string_lossy().to_string()).bind(&skill_path).bind(&manifest).bind(&checksum).bind(SKILL_BUNDLE_BUILDER_VERSION).bind(&runtime.descriptor().provider_label).bind(&runtime.descriptor().model).bind(&done).execute(pool).await?;
        sqlx::query("UPDATE artifact_bundle_jobs SET status='ready',stage='ready',runtime_version=?2,elapsed_ms=?3,updated_at=?4,finished_at=?4 WHERE job_id=?1").bind(&job_id).bind(run.runtime_version).bind((run.elapsed_ms + reconstruction_run.elapsed_ms) as i64).bind(&done).execute(pool).await?;
        // `output/` is now immutable content and its durable receipts are in
        // SQLite. The remaining workspace only held build inputs and the
        // vendored authoring guide, so it has no value once the bundle is ready.
        fs::remove_dir_all(&working).map_err(|error| InsightsError::Invalid(error.to_string()))?;
        ready_artifact_skill_bundle(pool, &artifact_id).await
    }.await;
    if let Err(error) = &outcome {
        let done = Utc::now().to_rfc3339();
        let message = error.to_string();
        let _ = sqlx::query("UPDATE artifact_bundle_jobs SET status='failed',stage='failed',error_code='build_failed',error_message=?2,updated_at=?3,finished_at=?3 WHERE job_id=?1").bind(&job_id).bind(message.chars().take(500).collect::<String>()).bind(done).execute(pool).await;
    }
    outcome
}

async fn run_phase<R: AiRuntime + ?Sized>(
    runtime: &R,
    prompt: &str,
    working_directory: PathBuf,
) -> Result<dystil_ai::AiAutomationRun> {
    // Provider event payloads are intentionally discarded. Draining prevents a
    // verbose automation stream from blocking either headless phase.
    let (events, mut receiver) = mpsc::channel(128);
    let drain = tokio::spawn(async move { while receiver.recv().await.is_some() {} });
    let run = runtime
        .run_automation(
            AiAutomationRequest {
                prompt: prompt.into(),
                working_directory,
                timeout: Duration::from_secs(15 * 60),
            },
            events,
        )
        .await
        .map_err(|error| InsightsError::Invalid(error.to_string()));
    let _ = drain.await;
    run
}

async fn update_stage(pool: &SqlitePool, job_id: &str, stage: &str) -> Result<()> {
    sqlx::query("UPDATE artifact_bundle_jobs SET stage=?2,updated_at=?3 WHERE job_id=?1 AND status='running'")
        .bind(job_id)
        .bind(stage)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn build_skill_bundle<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    artifact_id: &str,
    paths: &SkillBundlePaths,
) -> Result<SkillBundleView> {
    let (view, build) = start_skill_bundle_build(pool, runtime, artifact_id, paths).await?;
    match build {
        Some(build) => run_skill_bundle_build(pool, runtime, build, paths).await,
        None => Ok(view),
    }
}

/// Resolve a ready bundle's immutable skill directory and checksum. Installation
/// adapters use this narrow lookup so they never operate on an unvalidated job
/// workspace.
pub async fn ready_bundle_location(
    pool: &SqlitePool,
    bundle_id: &str,
) -> Result<(PathBuf, String, String)> {
    let row = sqlx::query("SELECT directory,skill_path,checksum FROM artifact_bundles WHERE bundle_id=?1 AND status='ready'")
        .bind(bundle_id).fetch_optional(pool).await?
        .ok_or_else(|| InsightsError::Invalid("skill bundle is not ready".into()))?;
    let directory: String = row.get("directory");
    let skill_path: String = row.get("skill_path");
    Ok((
        PathBuf::from(directory).join(skill_path),
        row.get("checksum"),
        row.get("directory"),
    ))
}

pub async fn record_skill_bundle_install(
    pool: &SqlitePool,
    bundle_id: &str,
    target: SkillInstallTarget,
    destination: &Path,
    checksum: &str,
) -> Result<SkillInstallReceipt> {
    let target_name = match target {
        SkillInstallTarget::Codex => "codex",
        SkillInstallTarget::Claude => "claude",
        SkillInstallTarget::ClaudeUpload => "claude_upload",
        SkillInstallTarget::Chatgpt => "chatgpt",
        SkillInstallTarget::Pi => "pi",
    };
    let destination = destination.to_string_lossy().to_string();
    let existing = sqlx::query("SELECT install_id,status FROM artifact_bundle_installs WHERE bundle_id=?1 AND target=?2 AND destination=?3")
        .bind(bundle_id).bind(target_name).bind(&destination).fetch_optional(pool).await?;
    let (install_id, status) = if let Some(row) = existing {
        (row.get("install_id"), row.get("status"))
    } else {
        let install_id = format!("sbi_{}", Uuid::new_v4().simple());
        let status = "installed".to_string();
        sqlx::query("INSERT INTO artifact_bundle_installs(install_id,bundle_id,target,destination,installed_checksum,status,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)")
            .bind(&install_id).bind(bundle_id).bind(target_name).bind(&destination).bind(checksum).bind(&status).bind(Utc::now().to_rfc3339()).execute(pool).await?;
        (install_id, status)
    };
    Ok(SkillInstallReceipt {
        install_id,
        bundle_id: bundle_id.into(),
        target,
        destination,
        status,
    })
}

pub async fn skill_bundle_installation_exists(
    pool: &SqlitePool,
    bundle_id: &str,
    target: SkillInstallTarget,
    destination: &Path,
) -> Result<bool> {
    let target_name = match target {
        SkillInstallTarget::Codex => "codex",
        SkillInstallTarget::Claude => "claude",
        SkillInstallTarget::ClaudeUpload => "claude_upload",
        SkillInstallTarget::Chatgpt => "chatgpt",
        SkillInstallTarget::Pi => "pi",
    };
    Ok(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artifact_bundle_installs WHERE bundle_id=?1 AND target=?2 AND destination=?3")
        .bind(bundle_id).bind(target_name).bind(destination.to_string_lossy().to_string()).fetch_one(pool).await? > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use dystil_ai::{
        AiAnswerRequest, AiAutomationRun, AiModelTier, AiRuntimeDescriptor, AiRuntimeError,
        AiRuntimeErrorCode, AiRuntimeEvent, AiRuntimeKind, AiStructuredRequest, AiStructuredRun,
        TeammateAnswerRun,
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct FixtureRuntime {
        descriptor: AiRuntimeDescriptor,
        fail: AtomicBool,
        calls: AtomicUsize,
    }

    impl FixtureRuntime {
        fn new() -> Self {
            Self {
                descriptor: AiRuntimeDescriptor {
                    kind: AiRuntimeKind::Codex,
                    provider_label: "test".into(),
                    model: "test-model".into(),
                },
                fail: AtomicBool::new(false),
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl AiRuntime for FixtureRuntime {
        fn descriptor(&self) -> &AiRuntimeDescriptor {
            &self.descriptor
        }

        fn model_for_tier(&self, _: AiModelTier) -> String {
            self.descriptor.model.clone()
        }

        async fn answer(
            &self,
            _: AiAnswerRequest,
        ) -> std::result::Result<TeammateAnswerRun, AiRuntimeError> {
            Err(AiRuntimeError::new(
                AiRuntimeErrorCode::Internal,
                "not used",
            ))
        }

        async fn infer_structured(
            &self,
            _: AiStructuredRequest,
        ) -> std::result::Result<AiStructuredRun, AiRuntimeError> {
            Err(AiRuntimeError::new(
                AiRuntimeErrorCode::Internal,
                "not used",
            ))
        }

        async fn run_automation(
            &self,
            request: AiAutomationRequest,
            _: mpsc::Sender<AiRuntimeEvent>,
        ) -> std::result::Result<dystil_ai::AiAutomationRun, AiRuntimeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                return Err(AiRuntimeError::new(
                    AiRuntimeErrorCode::Transport,
                    "simulated provider failure",
                ));
            }
            if request.prompt.contains("Workflow Reconstruction Agent") {
                fs::write(
                    request.working_directory.join("input/WORKFLOW.md"),
                    "# Workflow reconstruction\n\n## Task outcome and boundaries\nPrepare the approved document.\n\n## Trigger and starting state\nA source record is available.\n\n## Inputs and source discovery\nLocate the source material.\n\n## Systems, surfaces, and access\nUse available tools.\n\n## Observed end-to-end workflow\n1. Review the source.\n2. Prepare the document.\n\n## Decisions, variants, and exceptions\nUse runtime discovery when needed.\n\n## Outputs, destinations, and naming\nSave the document.\n\n## Validation and completion signals\nConfirm the saved output.\n\n## Runtime execution strategy\nUse connectors then local or browser tools.\n\n## Evidence map\n- Source review: frame:1\n\n## Unknowns and runtime discovery\nDiscover missing access at runtime.\n",
                )
                .unwrap();
            } else if request.prompt.contains("Skill Bundle Reviewer") {
                fs::write(
                    request.working_directory.join("input/BUNDLE_REVIEW.md"),
                    "# Bundle review\n\n## Verdict\napproved\n\n## Supported workflow mapping\n- Review the source — supported by Observed end-to-end workflow\n\n## Required corrections\nNone\n",
                )
                .unwrap();
            } else {
                write_valid_output(&request.working_directory.join("output"));
            }
            Ok(AiAutomationRun {
                runtime: AiRuntimeKind::Codex,
                runtime_version: Some("fixture".into()),
                elapsed_ms: 1,
                output: "done".into(),
            })
        }
    }

    async fn artifact_for_bundle() -> (tempfile::TempDir, SqlitePool, String) {
        let directory = tempfile::tempdir().unwrap();
        let pool = crate::open_insights_database(directory.path().join("insights.sqlite"))
            .await
            .unwrap();
        let finding_id = crate::test_support::seed_findings(&pool, 1).await.remove(0);
        let kept = crate::keep_finding(&pool, &finding_id, true).await.unwrap();
        (directory, pool, kept.artifact.artifact_id)
    }

    fn write_valid_output(root: &Path) {
        fs::create_dir_all(root.join("skill/purchase-order-review")).unwrap();
        fs::write(root.join("prompt.md"), "Review this purchase order.").unwrap();
        fs::create_dir_all(root.join("skill/purchase-order-review/references")).unwrap();
        fs::write(
            root.join("skill/purchase-order-review/references/workflow.md"),
            "Review inputs, validate terms, and produce an approval draft.",
        )
        .unwrap();
        fs::write(root.join("skill/purchase-order-review/SKILL.md"), "---\nname: purchase-order-review\ndescription: Review a purchase order when an approval request needs checking.\n---\n\nRead [the workflow](references/workflow.md), review inputs, validate terms, and produce an approval draft.").unwrap();
    }

    #[tokio::test]
    async fn reconstruction_treats_evidence_labels_as_best_effort_grounding() {
        let directory = tempfile::tempdir().unwrap();
        let workflow_path = directory.path().join("WORKFLOW.md");
        fs::write(
            &workflow_path,
            "# Workflow reconstruction\n\n## Task outcome and boundaries\nPrepare the approved document.\n\n## Trigger and starting state\nA source record is available.\n\n## Inputs and source discovery\nLocate the source material.\n\n## Systems, surfaces, and access\nUse available tools.\n\n## Observed end-to-end workflow\n1. Review the source.\n2. Prepare the document.\n\n## Decisions, variants, and exceptions\nUse runtime discovery when needed.\n\n## Outputs, destinations, and naming\nSave the document.\n\n## Validation and completion signals\nConfirm the saved output.\n\n## Runtime execution strategy\nUse connectors then local or browser tools.\n\n## Evidence map\n- E1: source record\n- frame:999999: related browser work\n\n## Unknowns and runtime discovery\nDiscover missing access at runtime.\n",
        )
        .unwrap();
        let pool = crate::open_insights_database(directory.path().join("insights.sqlite"))
            .await
            .unwrap();

        let (_, identifiers, _) = validate_reconstruction(&pool, &workflow_path, &BTreeSet::new())
            .await
            .unwrap();

        assert!(identifiers.contains("frame:999999"));
    }

    #[test]
    fn validates_a_portable_bundle_and_rejects_generated_reports() {
        let temp = tempfile::tempdir().unwrap();
        write_valid_output(temp.path());
        let (name, manifest, checksum, skill_path) = validate_output(temp.path()).unwrap();
        assert_eq!(name, "purchase-order-review");
        assert_eq!(skill_path, "skill/purchase-order-review");
        assert!(!manifest.is_empty());
        assert_eq!(checksum.len(), 64);
        fs::write(temp.path().join("README.md"), "forbidden").unwrap();
        assert!(validate_output(temp.path()).is_err());

        fs::remove_file(temp.path().join("README.md")).unwrap();
        fs::rename(
            temp.path().join("skill/purchase-order-review"),
            temp.path().join("skill/purchase-order-review-"),
        )
        .unwrap();
        assert!(validate_output(temp.path()).is_err());
    }

    #[test]
    fn rejects_files_outside_the_portable_bundle_layout() {
        let temp = tempfile::tempdir().unwrap();
        write_valid_output(temp.path());

        fs::write(temp.path().join("build-notes.txt"), "not an artifact").unwrap();
        assert!(validate_output(temp.path()).is_err());
        fs::remove_file(temp.path().join("build-notes.txt")).unwrap();

        let skill = temp.path().join("skill/purchase-order-review");
        fs::write(skill.join("README.md"), "not part of the skill").unwrap();
        assert!(validate_output(temp.path()).is_err());
        fs::remove_file(skill.join("README.md")).unwrap();

        fs::create_dir_all(skill.join("agents")).unwrap();
        fs::write(skill.join("agents/provider.json"), "{}").unwrap();
        assert!(validate_output(temp.path()).is_err());
    }

    #[test]
    fn validates_references_metadata_and_text_encoding() {
        let temp = tempfile::tempdir().unwrap();
        write_valid_output(temp.path());
        let skill = temp.path().join("skill/purchase-order-review");
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: purchase-order-review\ndescription: Review a purchase order when an approval request needs checking.\n---\n\nRead [the workflow](references/workflow.md) and use [the checklist](references/checklist.md).",
        )
        .unwrap();
        assert!(validate_output(temp.path()).is_err());

        fs::create_dir_all(skill.join("references")).unwrap();
        fs::write(
            skill.join("references/checklist.md"),
            "Check supplier and amount.",
        )
        .unwrap();
        assert!(validate_output(temp.path()).is_ok());

        fs::create_dir_all(skill.join("agents")).unwrap();
        fs::write(
            skill.join("agents/openai.yaml"),
            "asset: assets/missing.png",
        )
        .unwrap();
        assert!(validate_output(temp.path()).is_err());
        fs::create_dir_all(skill.join("assets")).unwrap();
        fs::write(skill.join("assets/missing.png"), [0_u8, 159, 255]).unwrap();
        assert!(validate_output(temp.path()).is_ok());

        fs::write(skill.join("references/not-text.md"), [0_u8, 159, 255]).unwrap();
        assert!(validate_output(temp.path()).is_err());
    }

    #[test]
    fn rejects_temporary_or_provider_specific_skill_instructions() {
        let temp = tempfile::tempdir().unwrap();
        write_valid_output(temp.path());
        let skill = temp.path().join("skill/purchase-order-review/SKILL.md");
        fs::write(
            &skill,
            "---\nname: purchase-order-review\ndescription: Review purchase orders.\n---\n\nRun $skill-purchase-order-review.",
        )
        .unwrap();
        assert!(validate_output(temp.path()).is_err());
        fs::write(
            &skill,
            "---\nname: purchase-order-review\ndescription: Review purchase orders.\n---\n\nRead builder/skill-creator/SKILL.md.",
        )
        .unwrap();
        assert!(validate_output(temp.path()).is_err());
    }

    #[test]
    fn portable_output_requires_workflow_reference_and_evidence_backed_urls() {
        let temp = tempfile::tempdir().unwrap();
        write_valid_output(temp.path());
        let allowed = BTreeSet::from(["https://portal.example.test/orders".to_string()]);
        fs::write(
            temp.path().join("prompt.md"),
            "Open https://portal.example.test/orders and review the order.",
        )
        .unwrap();
        assert!(validate_output_with_urls(temp.path(), &allowed).is_ok());
        fs::write(
            temp.path().join("prompt.md"),
            "Open https://invented.example.test/orders and review the order.",
        )
        .unwrap();
        assert!(validate_output_with_urls(temp.path(), &allowed).is_err());
        fs::remove_file(
            temp.path()
                .join("skill/purchase-order-review/references/workflow.md"),
        )
        .unwrap();
        assert!(validate_output_with_urls(temp.path(), &allowed).is_err());
    }

    #[test]
    fn production_profile_is_headless_and_best_effort() {
        for required in [
            "Do not ask the user questions",
            "Do not inspect screenshots",
            "Do not install the\ngenerated skill",
            "portable across Codex, Claude, ChatGPT, Claude/Cowork, and Pi",
            "connector-first source-discovery step",
            "Browser/computer use belongs to the generated artifact at runtime",
        ] {
            assert!(
                PROFILE.contains(required),
                "missing profile clause: {required}"
            );
        }
    }

    #[test]
    fn bundle_review_requires_a_concrete_verdict_and_corrections() {
        let temp = tempfile::tempdir().unwrap();
        let review = temp.path().join("BUNDLE_REVIEW.md");
        fs::write(
            &review,
            "# Bundle review\n\n## Verdict\nrewrite\n\n## Supported workflow mapping\n- Draft approval — supported by workflow\n\n## Required corrections\n- Remove the unobserved default approval recipient.\n",
        )
        .unwrap();
        let parsed = validate_bundle_review(&review).unwrap();
        assert_eq!(parsed.verdict, BundleReviewVerdict::Rewrite);
        assert!(parsed.corrections.contains("default approval recipient"));

        fs::write(
            &review,
            "# Bundle review\n\n## Verdict\nrewrite\n\n## Supported workflow mapping\n- Draft approval — supported by workflow\n\n## Required corrections\nNone\n",
        )
        .unwrap();
        assert!(validate_bundle_review(&review).is_err());
    }

    #[tokio::test]
    async fn build_is_immutable_idempotent_and_retryable() {
        let (directory, pool, artifact_id) = artifact_for_bundle().await;
        let paths = SkillBundlePaths::new(directory.path());
        let runtime = FixtureRuntime::new();

        let first = build_skill_bundle(&pool, &runtime, &artifact_id, &paths)
            .await
            .unwrap();
        assert_eq!(first.status, SkillBundleStatus::Ready);
        assert_eq!(runtime.calls.load(Ordering::SeqCst), 3);
        let reconstructions: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM artifact_workflow_reconstructions WHERE artifact_id=?1",
        )
        .bind(&artifact_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(reconstructions, 1);
        let second = build_skill_bundle(&pool, &runtime, &artifact_id, &paths)
            .await
            .unwrap();
        assert_eq!(first.bundle_id, second.bundle_id);
        assert_eq!(runtime.calls.load(Ordering::SeqCst), 3);
        assert!(paths
            .bundles_root
            .join(&artifact_id)
            .join("1/prompt.md")
            .is_file());
        assert!(
            !paths.builds_root.exists()
                || std::fs::read_dir(&paths.builds_root)
                    .unwrap()
                    .next()
                    .is_none()
        );

        let (_retry_directory, retry_pool, retry_artifact_id) = artifact_for_bundle().await;
        let retry_paths = SkillBundlePaths::new(directory.path().join("retry"));
        runtime.fail.store(true, Ordering::SeqCst);
        assert!(
            build_skill_bundle(&retry_pool, &runtime, &retry_artifact_id, &retry_paths)
                .await
                .is_err()
        );
        runtime.fail.store(false, Ordering::SeqCst);
        let retried = build_skill_bundle(&retry_pool, &runtime, &retry_artifact_id, &retry_paths)
            .await
            .unwrap();
        assert_eq!(retried.status, SkillBundleStatus::Ready);
    }

    #[tokio::test]
    async fn start_persists_one_job_before_the_background_runner_starts() {
        let (directory, pool, artifact_id) = artifact_for_bundle().await;
        let paths = SkillBundlePaths::new(directory.path());
        let runtime = FixtureRuntime::new();

        let (started, pending) = start_skill_bundle_build(&pool, &runtime, &artifact_id, &paths)
            .await
            .unwrap();
        assert_eq!(started.status, SkillBundleStatus::Running);
        assert_eq!(started.stage, Some(SkillBundleStage::Preparing));
        assert!(started.job_id.is_some());
        assert_eq!(runtime.calls.load(Ordering::SeqCst), 0);

        let (converged, duplicate) =
            start_skill_bundle_build(&pool, &runtime, &artifact_id, &paths)
                .await
                .unwrap();
        assert_eq!(converged.job_id, started.job_id);
        assert!(duplicate.is_none());

        let finished = run_skill_bundle_build(&pool, &runtime, pending.unwrap(), &paths)
            .await
            .unwrap();
        assert_eq!(finished.status, SkillBundleStatus::Ready);
        assert_eq!(runtime.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn interrupted_build_is_marked_retryable_and_starts_fresh() {
        let (directory, pool, artifact_id) = artifact_for_bundle().await;
        let paths = SkillBundlePaths::new(directory.path());
        let runtime = FixtureRuntime::new();

        let (started, pending) = start_skill_bundle_build(&pool, &runtime, &artifact_id, &paths)
            .await
            .unwrap();
        assert!(pending.is_some());
        assert_eq!(started.status, SkillBundleStatus::Running);

        assert_eq!(
            interrupt_abandoned_skill_bundle_builds(&pool)
                .await
                .unwrap(),
            1
        );
        let interrupted = ready_artifact_skill_bundle(&pool, &artifact_id)
            .await
            .unwrap();
        assert_eq!(interrupted.status, SkillBundleStatus::Interrupted);
        assert_eq!(
            interrupted.error_message.as_deref(),
            Some("Dystil was closed before this skill finished building.")
        );

        let (retried, fresh_pending) =
            start_skill_bundle_build(&pool, &runtime, &artifact_id, &paths)
                .await
                .unwrap();
        assert_eq!(retried.status, SkillBundleStatus::Running);
        assert_ne!(retried.job_id, started.job_id);
        assert!(fresh_pending.is_some());
    }

    #[tokio::test]
    async fn safe_progress_stage_is_durable_and_exposes_no_provider_payload() {
        let (directory, pool, artifact_id) = artifact_for_bundle().await;
        let paths = SkillBundlePaths::new(directory.path());
        let runtime = FixtureRuntime::new();
        let (started, _pending) = start_skill_bundle_build(&pool, &runtime, &artifact_id, &paths)
            .await
            .unwrap();
        let job_id = started.job_id.unwrap();

        update_stage(&pool, &job_id, "investigating").await.unwrap();
        let stage: String =
            sqlx::query_scalar("SELECT stage FROM artifact_bundle_jobs WHERE job_id=?1")
                .bind(&job_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stage, "investigating");
        assert_eq!(
            ready_artifact_skill_bundle(&pool, &artifact_id)
                .await
                .unwrap()
                .stage,
            Some(SkillBundleStage::Investigating)
        );
    }
}

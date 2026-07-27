use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Duration;
use clap::{Parser, Subcommand};
use dystil_work_cards::{
    atom_json_schema, build_atom_prompt, build_card_prompt_from_atoms, build_evidence_windows,
    build_work_card_prompt, chunk_reduced_window, compact_window, merge_atoms,
    reduce_window_before_budget, sanitize_work_card, validate_atoms, validate_work_card,
    ChunkConfig, CompactedWindow, CompactionConfig, DistilledEvidenceChunk, EvidenceChunk,
    ExportedSegment, GeneratedAtoms, GeneratedWorkCard, MergedAtoms, PreBudgetReductionConfig,
    PromptConfig, PromptRecord, ReducedEvidenceWindow, ValidationReport, WindowConfig,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Parser)]
#[command(name = "work-card-eval")]
#[command(about = "Reproducible local work-card evaluation harness")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Reduce {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 160)]
        max_item_tokens: usize,
        #[arg(long, default_value_t = 5)]
        inactivity_minutes: i64,
        #[arg(long, default_value_t = 15)]
        max_duration_minutes: i64,
    },
    Chunk {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 4_000)]
        target_tokens: u32,
        #[arg(long, default_value_t = 6_000)]
        hard_max_tokens: u32,
        #[arg(long, default_value_t = 400)]
        overlap_tokens: u32,
    },
    AtomPrompts {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    ValidateAtoms {
        #[arg(long)]
        chunks: PathBuf,
        #[arg(long)]
        generated: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    MergeAtoms {
        #[arg(long)]
        generated: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    CardPromptsFromAtoms {
        #[arg(long)]
        reduced: PathBuf,
        #[arg(long)]
        atoms: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Compact {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 4_000)]
        max_tokens: u32,
        #[arg(long, default_value_t = 160)]
        max_item_tokens: usize,
        #[arg(long, default_value_t = 5)]
        inactivity_minutes: i64,
        #[arg(long, default_value_t = 15)]
        max_duration_minutes: i64,
    },
    Prompts {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Validate {
        #[arg(long)]
        compacted: PathBuf,
        #[arg(long)]
        generated: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Sanitize {
        #[arg(long)]
        compacted: PathBuf,
        #[arg(long)]
        generated: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Persist {
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        compacted: PathBuf,
        #[arg(long)]
        generated: PathBuf,
    },
}

#[derive(Debug, Serialize)]
struct CorpusStats {
    source_segments: usize,
    windows: usize,
    source_items: usize,
    kept_items: usize,
    duplicate_items: usize,
    source_tokens: u64,
    compacted_tokens: u64,
    compression_ratio: f64,
    truncated_windows: usize,
    p50_source_tokens: u32,
    p90_source_tokens: u32,
    p50_compacted_tokens: u32,
    p90_compacted_tokens: u32,
}

#[derive(Debug, Serialize)]
struct ValidationRecord {
    window_id: String,
    model_id: String,
    report: ValidationReport,
}

#[derive(Debug, Serialize)]
struct AtomPromptRecord {
    window_id: String,
    chunk_id: String,
    prompt: String,
    schema: serde_json::Value,
    evidence: Vec<dystil_work_cards::CompactedEvidence>,
}

#[derive(Debug, Serialize, serde::Deserialize)]
struct AtomValidationRecord {
    window_id: String,
    chunk_id: String,
    report: dystil_work_cards::AtomValidationReport,
    atoms: DistilledEvidenceChunk,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Reduce {
            input,
            output,
            max_item_tokens,
            inactivity_minutes,
            max_duration_minutes,
        } => reduce(
            &input,
            &output,
            max_item_tokens,
            inactivity_minutes,
            max_duration_minutes,
        ),
        Command::Chunk {
            input,
            output,
            target_tokens,
            hard_max_tokens,
            overlap_tokens,
        } => chunk(
            &input,
            &output,
            ChunkConfig {
                target_tokens,
                hard_max_tokens,
                overlap_tokens,
            },
        ),
        Command::AtomPrompts { input, output } => atom_prompts(&input, &output),
        Command::ValidateAtoms {
            chunks,
            generated,
            output,
        } => validate_atoms_file(&chunks, &generated, &output),
        Command::MergeAtoms { generated, output } => merge_atoms_file(&generated, &output),
        Command::CardPromptsFromAtoms {
            reduced,
            atoms,
            output,
        } => card_prompts_from_atoms(&reduced, &atoms, &output),
        Command::Compact {
            input,
            output,
            max_tokens,
            max_item_tokens,
            inactivity_minutes,
            max_duration_minutes,
        } => compact(
            &input,
            &output,
            CompactionConfig {
                max_tokens,
                max_item_tokens,
                ..CompactionConfig::default()
            },
            WindowConfig {
                inactivity: Duration::minutes(inactivity_minutes),
                max_duration: Duration::minutes(max_duration_minutes),
            },
        ),
        Command::Prompts { input, output } => prompts(&input, &output),
        Command::Validate {
            compacted,
            generated,
            output,
        } => validate(&compacted, &generated, &output),
        Command::Sanitize {
            compacted,
            generated,
            output,
        } => sanitize(&compacted, &generated, &output),
        Command::Persist {
            database,
            compacted,
            generated,
        } => persist(&database, &compacted, &generated),
    }
}

fn reduce(
    input: &Path,
    output: &Path,
    max_item_tokens: usize,
    inactivity_minutes: i64,
    max_duration_minutes: i64,
) -> Result<()> {
    let segments: Vec<ExportedSegment> = read_jsonl(input)?;
    let windows = build_evidence_windows(
        segments,
        &WindowConfig {
            inactivity: Duration::minutes(inactivity_minutes),
            max_duration: Duration::minutes(max_duration_minutes),
        },
    );
    let records = windows
        .iter()
        .map(|window| {
            reduce_window_before_budget(
                window,
                &PreBudgetReductionConfig {
                    max_item_tokens,
                    ..Default::default()
                },
            )
        })
        .collect::<Vec<_>>();
    let summary = serde_json::json!({"windows":records.len(),"remaining_items":records.iter().map(|r|r.stats.remaining_items).sum::<usize>(),"remaining_estimated_tokens":records.iter().map(|r|r.stats.remaining_estimated_tokens as u64).sum::<u64>()});
    write_jsonl(output, &records)?;
    println!("{}", summary);
    Ok(())
}

fn chunk(input: &Path, output: &Path, config: ChunkConfig) -> Result<()> {
    let records: Vec<ReducedEvidenceWindow> = read_jsonl(input)?;
    let mut chunks = Vec::new();
    let mut input_tokens = 0u64;
    for record in &records {
        let (mut made, stats) = chunk_reduced_window(record, &config);
        input_tokens += stats.total_input_tokens as u64;
        chunks.append(&mut made);
    }
    write_jsonl(output, &chunks)?;
    println!(
        "{}",
        serde_json::json!({"windows":records.len(),"chunks":chunks.len(),"estimated_pass1_input_tokens":input_tokens})
    );
    Ok(())
}

fn atom_prompts(input: &Path, output: &Path) -> Result<()> {
    let chunks: Vec<EvidenceChunk> = read_jsonl(input)?;
    let records = chunks
        .iter()
        .map(|chunk| AtomPromptRecord {
            window_id: chunk.window_id.clone(),
            chunk_id: chunk.chunk_id.clone(),
            prompt: build_atom_prompt(chunk),
            schema: atom_json_schema(chunk),
            evidence: chunk.evidence.clone(),
        })
        .collect::<Vec<_>>();
    write_jsonl(output, &records)
}

fn validate_atoms_file(chunks: &Path, generated: &Path, output: &Path) -> Result<()> {
    let chunks: Vec<EvidenceChunk> = read_jsonl(chunks)?;
    let lookup = chunks
        .into_iter()
        .map(|c| (c.chunk_id.clone(), c))
        .collect::<std::collections::HashMap<_, _>>();
    let generated: Vec<GeneratedAtoms> = read_jsonl(generated)?;
    let mut records = Vec::new();
    for mut record in generated {
        let chunk = lookup
            .get(&record.chunk_id)
            .with_context(|| format!("unknown chunk {}", record.chunk_id))?;
        let report = validate_atoms(chunk, &mut record.atoms);
        records.push(AtomValidationRecord {
            window_id: record.window_id,
            chunk_id: record.chunk_id,
            report,
            atoms: record.atoms,
        });
    }
    write_jsonl(output, &records)
}

fn merge_atoms_file(generated: &Path, output: &Path) -> Result<()> {
    let records: Vec<AtomValidationRecord> = read_jsonl(generated)?;
    let mut by_window = std::collections::BTreeMap::<String, Vec<DistilledEvidenceChunk>>::new();
    for record in records {
        by_window
            .entry(record.window_id)
            .or_default()
            .push(record.atoms);
    }
    let merged = by_window
        .into_iter()
        .map(|(id, chunks)| merge_atoms(id, chunks))
        .collect::<Vec<_>>();
    write_jsonl(output, &merged)
}

fn card_prompts_from_atoms(reduced: &Path, atoms: &Path, output: &Path) -> Result<()> {
    let reduced: Vec<ReducedEvidenceWindow> = read_jsonl(reduced)?;
    let atoms: Vec<MergedAtoms> = read_jsonl(atoms)?;
    let lookup = atoms
        .into_iter()
        .map(|a| (a.window_id.clone(), a))
        .collect::<std::collections::HashMap<_, _>>();
    let prompts = reduced
        .into_iter()
        .map(|record| {
            let atoms = lookup
                .get(&record.window.window_id)
                .with_context(|| format!("missing atoms {}", record.window.window_id))?;
            Ok(PromptRecord {
                window_id: record.window.window_id.clone(),
                prompt: build_card_prompt_from_atoms(&record.window, atoms),
                evidence: record.evidence,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    write_jsonl(output, &prompts)
}

fn persist(database: &Path, compacted: &Path, generated: &Path) -> Result<()> {
    let compacted: Vec<CompactedWindow> = read_jsonl(compacted)?;
    let mut generated: Vec<GeneratedWorkCard> = read_jsonl(generated)?;
    let lookup = compacted
        .into_iter()
        .map(|record| (record.window.window_id.clone(), record))
        .collect::<std::collections::HashMap<_, _>>();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let pool = dystil_storage::open_capture_database(database).await?;
        for generated in &mut generated {
            let compacted = lookup
                .get(&generated.window_id)
                .with_context(|| format!("unknown window {}", generated.window_id))?;
            sanitize_work_card(&mut generated.card, &compacted.evidence);
            let validation = validate_work_card(&generated.card, &compacted.evidence);
            if !validation.valid {
                anyhow::bail!(
                    "refusing to persist invalid card {}: {} validation errors",
                    generated.window_id,
                    validation.errors.len()
                );
            }
            let card_json = serde_json::to_value(&generated.card)?;
            let source_bytes = serde_json::to_vec(&compacted.evidence)?;
            let source_hash = format!("sha256:{}", hex::encode(Sha256::digest(source_bytes)));
            let status = serde_json::to_value(&generated.card.status)?
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            dystil_storage::upsert_work_card(
                &pool,
                &dystil_storage::NewWorkCard {
                    window_id: generated.window_id.clone(),
                    start_time: compacted.window.start_time.to_rfc3339(),
                    end_time: compacted.window.end_time.to_rfc3339(),
                    close_reason: compacted.window.close_reason.clone(),
                    title: generated.card.title.clone(),
                    summary: generated.card.summary.text.clone(),
                    applications: generated.card.applications.clone(),
                    artifacts: serde_json::to_value(&generated.card.artifacts)?,
                    actions: serde_json::to_value(&generated.card.actions)?,
                    last_observed_state: generated.card.last_observed_state.text.clone(),
                    status,
                    uncertainties: generated.card.uncertainties.clone(),
                    card_json,
                    model_id: generated.model_id.clone(),
                    source_hash,
                    embedding_model_id: None,
                    embedding: None,
                },
            )
            .await?;
        }
        println!(
            "{}",
            serde_json::json!({"persisted": generated.len(), "database": database})
        );
        Result::<()>::Ok(())
    })
}

fn sanitize(compacted: &Path, generated: &Path, output: &Path) -> Result<()> {
    let compacted: Vec<CompactedWindow> = read_jsonl(compacted)?;
    let mut generated: Vec<GeneratedWorkCard> = read_jsonl(generated)?;
    let lookup = compacted
        .into_iter()
        .map(|record| (record.window.window_id, record.evidence))
        .collect::<std::collections::HashMap<_, _>>();
    for record in &mut generated {
        let evidence = lookup
            .get(&record.window_id)
            .with_context(|| format!("unknown window {}", record.window_id))?;
        sanitize_work_card(&mut record.card, evidence);
    }
    write_jsonl(output, &generated)
}

fn compact(
    input: &Path,
    output: &Path,
    compaction: CompactionConfig,
    windowing: WindowConfig,
) -> Result<()> {
    let segments: Vec<ExportedSegment> = read_jsonl(input)?;
    let source_segments = segments.len();
    let windows = build_evidence_windows(segments, &windowing);
    let records = windows
        .into_iter()
        .map(|window| {
            let (evidence, stats) = compact_window(&window, &compaction);
            CompactedWindow {
                window,
                evidence,
                stats,
            }
        })
        .collect::<Vec<_>>();
    write_jsonl(output, &records)?;

    let mut source_per_window = records
        .iter()
        .map(|record| record.stats.source_estimated_tokens)
        .collect::<Vec<_>>();
    let mut compacted_per_window = records
        .iter()
        .map(|record| record.stats.compacted_estimated_tokens)
        .collect::<Vec<_>>();
    source_per_window.sort_unstable();
    compacted_per_window.sort_unstable();
    let source_tokens: u64 = source_per_window.iter().map(|value| *value as u64).sum();
    let compacted_tokens: u64 = compacted_per_window.iter().map(|value| *value as u64).sum();
    let stats = CorpusStats {
        source_segments,
        windows: records.len(),
        source_items: records.iter().map(|record| record.stats.source_items).sum(),
        kept_items: records.iter().map(|record| record.stats.kept_items).sum(),
        duplicate_items: records
            .iter()
            .map(|record| record.stats.duplicate_items)
            .sum(),
        source_tokens,
        compacted_tokens,
        compression_ratio: if source_tokens == 0 {
            1.0
        } else {
            compacted_tokens as f64 / source_tokens as f64
        },
        truncated_windows: records
            .iter()
            .filter(|record| record.stats.truncated)
            .count(),
        p50_source_tokens: percentile(&source_per_window, 0.50),
        p90_source_tokens: percentile(&source_per_window, 0.90),
        p50_compacted_tokens: percentile(&compacted_per_window, 0.50),
        p90_compacted_tokens: percentile(&compacted_per_window, 0.90),
    };
    println!("{}", serde_json::to_string_pretty(&stats)?);
    Ok(())
}

fn prompts(input: &Path, output: &Path) -> Result<()> {
    let records: Vec<CompactedWindow> = read_jsonl(input)?;
    let prompt_config = PromptConfig::default();
    let prompts = records
        .into_iter()
        .map(|record| PromptRecord {
            window_id: record.window.window_id.clone(),
            prompt: build_work_card_prompt(&record.window, &record.evidence, &prompt_config),
            evidence: record.evidence,
        })
        .collect::<Vec<_>>();
    write_jsonl(output, &prompts)
}

fn validate(compacted: &Path, generated: &Path, output: &Path) -> Result<()> {
    let compacted: Vec<CompactedWindow> = read_jsonl(compacted)?;
    let generated: Vec<GeneratedWorkCard> = read_jsonl(generated)?;
    let lookup = compacted
        .into_iter()
        .map(|record| (record.window.window_id, record.evidence))
        .collect::<std::collections::HashMap<_, _>>();
    let records = generated
        .into_iter()
        .map(|generated| {
            let evidence = lookup
                .get(&generated.window_id)
                .with_context(|| format!("unknown window {}", generated.window_id))?;
            Ok(ValidationRecord {
                window_id: generated.window_id,
                model_id: generated.model_id,
                report: validate_work_card(&generated.card, evidence),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    write_jsonl(output, &records)
}

fn read_jsonl<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    BufReader::new(file)
        .lines()
        .enumerate()
        .filter_map(|(index, line)| match line {
            Ok(line) if line.trim().is_empty() => None,
            result => Some((index, result)),
        })
        .map(|(index, line)| {
            let line = line.with_context(|| format!("reading line {}", index + 1))?;
            serde_json::from_str(&line)
                .with_context(|| format!("parsing {} line {}", path.display(), index + 1))
        })
        .collect()
}

fn write_jsonl<T: Serialize>(path: &Path, records: &[T]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    for record in records {
        serde_json::to_writer(&mut writer, record)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn percentile(values: &[u32], fraction: f64) -> u32 {
    if values.is_empty() {
        return 0;
    }
    let index = ((values.len() - 1) as f64 * fraction).round() as usize;
    values[index]
}

use crate::{CompactedEvidence, DistilledEvidenceChunk, EvidenceChunk};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AtomValidationReport {
    pub emitted: usize,
    pub accepted: usize,
    pub rejected: usize,
    pub invalid_citations: usize,
    pub sanitized_fields: usize,
    pub cited_evidence_ids: Vec<String>,
}

pub fn validate_atoms(
    chunk: &EvidenceChunk,
    output: &mut DistilledEvidenceChunk,
) -> AtomValidationReport {
    let known = chunk
        .evidence
        .iter()
        .map(|e| (e.evidence_id.as_str(), e))
        .collect::<HashMap<_, _>>();
    let mut report = AtomValidationReport {
        emitted: output.atoms.len(),
        ..Default::default()
    };
    let mut cited = BTreeSet::new();
    let mut retained = Vec::new();
    for mut atom in std::mem::take(&mut output.atoms) {
        let ids = atom
            .evidence_ids
            .iter()
            .filter(|id| known.contains_key(id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if ids.is_empty() || ids.len() != atom.evidence_ids.len() {
            report.rejected += 1;
            report.invalid_citations += 1;
            continue;
        }
        atom.evidence_ids = ids;
        let cited_items = atom
            .evidence_ids
            .iter()
            .filter_map(|id| known.get(id.as_str()))
            .collect::<Vec<_>>();
        let earliest = cited_items.iter().map(|e| e.occurred_at).min().unwrap();
        let latest = cited_items.iter().map(|e| e.occurred_at).max().unwrap();
        if atom.occurred_at < earliest || atom.occurred_at > latest {
            report.rejected += 1;
            continue;
        }
        if atom.action.trim().is_empty() || atom.action.len() > 300 {
            report.rejected += 1;
            continue;
        }
        if let Some(app) = &atom.application {
            if !cited_items.iter().any(|e| {
                e.app_name
                    .as_deref()
                    .is_some_and(|x| x.eq_ignore_ascii_case(app))
            }) {
                atom.application = None;
                report.sanitized_fields += 1;
            }
        }
        sanitize_grounded(&mut atom.object, &cited_items, &mut report);
        sanitize_grounded(&mut atom.result, &cited_items, &mut report);
        sanitize_grounded(&mut atom.state_before, &cited_items, &mut report);
        sanitize_grounded(&mut atom.state_after, &cited_items, &mut report);
        atom.action = dystil_redact::sanitize_text(&atom.action);
        if secret_like(&atom.action) {
            report.rejected += 1;
            continue;
        }
        cited.extend(atom.evidence_ids.iter().cloned());
        retained.push(atom);
    }
    output.atoms = retained;
    report.accepted = output.atoms.len();
    report.cited_evidence_ids = cited.into_iter().collect();
    report
}
fn sanitize_grounded(
    field: &mut Option<String>,
    cited: &[&&CompactedEvidence],
    report: &mut AtomValidationReport,
) {
    if let Some(value) = field {
        let redacted = dystil_redact::sanitize_text(value);
        let supported = cited.iter().any(|e| {
            let h = format!(
                "{}\n{}\n{}\n{}",
                e.text,
                e.app_name.as_deref().unwrap_or_default(),
                e.window_name.as_deref().unwrap_or_default(),
                e.browser_url.as_deref().unwrap_or_default()
            )
            .to_lowercase();
            h.contains(&redacted.to_lowercase())
        });
        if !supported || secret_like(&redacted) {
            *field = None;
            report.sanitized_fields += 1;
        } else {
            *value = redacted;
        }
    }
}
fn secret_like(value: &str) -> bool {
    value.contains("[PASSWORD]") || value.contains("[API_KEY]") || value.contains("[SECRET]")
}

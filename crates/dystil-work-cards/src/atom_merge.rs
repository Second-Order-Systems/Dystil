use crate::{DistilledAtom, DistilledEvidenceChunk, MergedAtoms};
use std::collections::HashSet;

pub fn merge_atoms(window_id: String, chunks: Vec<DistilledEvidenceChunk>) -> MergedAtoms {
    let mut all = chunks.into_iter().flat_map(|c| c.atoms).collect::<Vec<_>>();
    all.sort_by(|a, b| {
        a.occurred_at
            .cmp(&b.occurred_at)
            .then_with(|| a.atom_id.cmp(&b.atom_id))
    });
    let mut seen = HashSet::new();
    let mut merged: Vec<DistilledAtom> = Vec::new();
    for atom in all {
        let key = format!(
            "{:?}|{:?}|{:?}|{}",
            atom.event_type,
            atom.application,
            atom.object,
            atom.action.to_lowercase()
        );
        if !seen.insert(key) {
            if let Some(prev) = merged.last_mut() {
                for id in atom.evidence_ids {
                    if !prev.evidence_ids.contains(&id) {
                        prev.evidence_ids.push(id)
                    }
                }
            }
        } else {
            merged.push(atom);
        }
    }
    MergedAtoms {
        window_id,
        atoms: merged,
        uncertainties: Vec::new(),
    }
}

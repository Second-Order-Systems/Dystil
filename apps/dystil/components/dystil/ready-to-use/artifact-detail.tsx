"use client";

import type { ArtifactChangePreview, ReadyArtifactDetail, WorthFixingEvidenceLine } from "@/lib/utils/tauri";
import { ArtifactChange } from "./artifact-change";

type Props = { detail: ReadyArtifactDetail; provenance: WorthFixingEvidenceLine[] | null; preview: ArtifactChangePreview | null; pending: boolean; onClose: () => void; onProvenance: () => void; onPropose: (request: string) => Promise<boolean>; onRetry: () => void; onConfirm: () => void; onReject: () => void; onRemove: () => void };

export function ArtifactDetail({ detail, provenance, preview, pending, onClose, onProvenance, onPropose, onRetry, onConfirm, onReject, onRemove }: Props) {
  const prompt = detail.card.kind === "prompt" || detail.card.kind === "saved_prompt";
  return <section aria-label={`${detail.card.title} details`} className="mt-5 rounded-[12px] bg-[#f5f8f6] px-5 py-5">
    <div className="flex items-start justify-between gap-4"><div><p className="text-[13px] text-[#68706c]">Kept {new Date(detail.keptAt).toLocaleDateString()}</p><h3 className="mt-1 text-[19px] font-medium text-[#171a18]">{detail.card.title}</h3></div><button onClick={onClose} className="text-[14px] text-[#626965]" aria-label="Close artifact details">Close</button></div>
    <div className={`mt-4 max-w-[75ch] whitespace-pre-wrap text-[15px] leading-7 text-[#343a37] ${prompt ? "font-mono" : ""}`}>{detail.body}</div>
    <div className="mt-5 flex flex-wrap gap-5"><button aria-expanded={provenance !== null} onClick={onProvenance} className="text-[14px] font-medium text-[#0f6e56]">Where this came from</button><button onClick={onRemove} className="text-[14px] text-[#8b3028]">Remove</button></div>
    {provenance ? <section aria-label="Artifact provenance" className="mt-4 border-t border-[#d9dedb] pt-4">{detail.provenanceAvailable ? <ul className="grid gap-3">{provenance.map((line) => <li key={line.evidenceId} className="text-[13px] leading-5 text-[#58605c]">{line.available ? line.description : "This evidence is no longer available"}<span className="block text-[12px] text-[#7b827e]">{line.app ? `${line.app} · ` : ""}{new Date(line.occurredAt).toLocaleString()}</span></li>)}</ul> : <p className="text-[14px] text-[#68706c]">The source evidence is no longer available. Your kept artifact is unchanged.</p>}</section> : null}
    {detail.changes.length ? <details className="mt-5"><summary className="cursor-pointer text-[13px] text-[#68706c]">Changes ({detail.changeCount})</summary><ul className="mt-2 grid gap-2">{detail.changes.map((change) => <li key={`${change.changedAt}-${change.request}`} className="text-[13px] text-[#59615d]">{change.request} · {new Date(change.changedAt).toLocaleDateString()}</li>)}</ul></details> : null}
    <ArtifactChange detail={detail} preview={preview} pending={pending} onPropose={onPropose} onRetry={onRetry} onConfirm={onConfirm} onReject={onReject} />
  </section>;
}

"use client";

import type {
  ArtifactChangePreview,
  ReadyArtifactCard as Card,
  ReadyArtifactDetail,
  WorthFixingEvidenceLine,
} from "@/lib/utils/tauri";
import { ArtifactDetail } from "./artifact-detail";

const actionLabel = {
  copy: "Copy it",
  open: "Open it",
  share: "Send to someone",
  show_how: "Show me how",
} as const;

type Props = {
  artifact: Card;
  detail: ReadyArtifactDetail | null;
  provenance: WorthFixingEvidenceLine[] | null;
  preview: ArtifactChangePreview | null;
  pending: boolean;
  onAction: (action: Card["primaryAction"] | Card["secondaryAction"]) => void;
  onClose: () => void;
  onProvenance: () => void;
  onPropose: (request: string) => Promise<boolean>;
  onRetry: () => void;
  onConfirm: () => void;
  onReject: () => void;
  onRemove: () => void;
};

export function ArtifactCard({ artifact, detail, provenance, preview, pending, onAction, onClose, onProvenance, onPropose, onRetry, onConfirm, onReject, onRemove }: Props) {
  const expanded = detail?.card.artifactId === artifact.artifactId;
  const kind = artifact.kind === "existing_capability" ? "Tool you already have" : artifact.kind === "runbook" ? "Runbook" : "Prompt";

  return <article tabIndex={-1} className="border-b border-[#deddd8] py-6 first:border-t focus:outline-none">
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_auto] xl:gap-6">
      <div>
        <p className="text-[13px] font-medium text-[#0f6e56]">{kind}</p>
        <h2 className="mt-2 text-[21px] font-medium leading-[1.35] tracking-[-0.02em] text-[#151716]">{artifact.title}</h2>
        <p className="mt-2 max-w-[68ch] text-[15px] leading-6 text-[#59615d]">{artifact.description}</p>
        <p className="mt-2 text-[12px] text-[#7a817d]">{artifact.lastUsedAt ? `Last used ${new Date(artifact.lastUsedAt).toLocaleString()}` : "Not used yet"}</p>
      </div>
      <div className="flex flex-wrap items-start gap-3 xl:justify-end">
        <button disabled={pending} onClick={() => onAction(artifact.primaryAction)} className="rounded-[9px] bg-[#087b5f] px-4 py-2 text-[14px] font-medium text-white disabled:bg-[#cbd4d0] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#087b5f]">{pending ? "Working…" : actionLabel[artifact.primaryAction]}</button>
        <button aria-expanded={expanded && artifact.secondaryAction !== "share"} disabled={pending} onClick={() => onAction(artifact.secondaryAction)} className="rounded-[9px] border border-[#cfcec8] bg-white px-4 py-2 text-[14px] font-medium text-[#4c534f] hover:border-[#8db7a8] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#087b5f]">{actionLabel[artifact.secondaryAction]}</button>
      </div>
    </div>
    {expanded ? <ArtifactDetail detail={detail} provenance={provenance} preview={preview} pending={pending} onClose={onClose} onProvenance={onProvenance} onPropose={onPropose} onRetry={onRetry} onConfirm={onConfirm} onReject={onReject} onRemove={onRemove} /> : null}
  </article>;
}

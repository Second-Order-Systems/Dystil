"use client";

import { useRef, useState } from "react";
import type { DispositionKind, WorthFixingCard as Card } from "@/lib/utils/tauri";
import { FindingEvidence } from "./finding-evidence";

type Props = {
  finding: Card;
  pending: boolean;
  onKeep: (id: string) => void;
  onDismiss: (id: string, reason: DispositionKind) => void;
  onCorrect: (id: string, correction: string, intent: string) => Promise<boolean>;
};

export function FindingCard({ finding, pending, onKeep, onDismiss, onCorrect }: Props) {
  const [evidenceOpen, setEvidenceOpen] = useState(false);
  const [correcting, setCorrecting] = useState(false);
  const [correction, setCorrection] = useState("");
  const input = useRef<HTMLTextAreaElement>(null);

  return <article tabIndex={-1} className="border-b border-[#deddd8] py-7 first:border-t focus:outline-none">
    <p className="text-[14px] font-medium text-[#0f6e56]">{finding.label}</p>
    <h2 className="mt-2 max-w-[72ch] text-[23px] font-medium leading-[1.35] tracking-[-0.02em] text-[#151716]">{finding.claim}</h2>
    <p className="mt-3 max-w-[72ch] text-[16px] leading-7 text-[#58605c]">{finding.whyWorthFixing}</p>
    <div className="mt-4 rounded-[12px] bg-[#f4f8f6] px-4 py-3">
      <p className="text-[14px] font-medium text-[#26312d]">{finding.handoffTitle}</p>
      <p className="mt-1 text-[14px] leading-6 text-[#5b635f]">{finding.handoffPreview}</p>
    </div>
    <div className="mt-5 flex flex-wrap items-center gap-x-5 gap-y-3">
      <button type="button" disabled={pending || !finding.evidenceAvailable} onClick={() => onKeep(finding.findingId)} className="rounded-[10px] bg-[#087b5f] px-5 py-2.5 text-[15px] font-medium text-white hover:bg-[#06634d] disabled:cursor-not-allowed disabled:bg-[#cbd4d0] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-3 focus-visible:outline-[#087b5f]">{pending ? "Keeping…" : "Keep this"}</button>
      <button type="button" aria-expanded={evidenceOpen} onClick={() => setEvidenceOpen((value) => !value)} className="text-[14px] font-medium text-[#0f6e56] hover:text-[#094b3b] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#087b5f]">{evidenceOpen ? "Hide what Dystil saw" : "Show me what you saw"}</button>
      <button type="button" aria-expanded={correcting} onClick={() => { setCorrecting((value) => !value); queueMicrotask(() => input.current?.focus()); }} className="text-[14px] text-[#5f6662] hover:text-[#202522]">This is close, but…</button>
      <details className="relative"><summary className="cursor-pointer list-none text-[14px] text-[#6c726f] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#087b5f]">Dismiss ▾</summary><div className="absolute right-0 z-10 mt-2 w-44 rounded-[10px] border border-[#d9d7d1] bg-white p-1 shadow-[0_8px_24px_rgba(20,30,26,0.12)]"><button className="block w-full rounded-[7px] px-3 py-2 text-left text-[13px] hover:bg-[#f4f6f4]" onClick={() => onDismiss(finding.findingId, "not_a_problem")}>Not a problem</button><button className="block w-full rounded-[7px] px-3 py-2 text-left text-[13px] hover:bg-[#f4f6f4]" onClick={() => onDismiss(finding.findingId, "leave_it")}>I know, leave it</button></div></details>
    </div>
    {correcting ? <form className="mt-5 max-w-[720px] rounded-[12px] border border-[#d8ded9] p-4" onSubmit={async (event) => { event.preventDefault(); if (await onCorrect(finding.findingId, correction, "improve_finding")) setCorrecting(false); }}><label className="text-[14px] font-medium text-[#2b312e]" htmlFor={`correction-${finding.findingId}`}>What should Dystil understand differently?</label><textarea ref={input} required maxLength={2000} id={`correction-${finding.findingId}`} value={correction} onChange={(event) => setCorrection(event.target.value)} className="mt-2 min-h-24 w-full resize-y rounded-[9px] border border-[#cfcec8] bg-white p-3 text-[14px] leading-6 outline-none focus:border-[#087b5f]" /><div className="mt-3 flex gap-4"><button disabled={pending} className="rounded-[9px] bg-[#087b5f] px-4 py-2 text-[14px] font-medium text-white disabled:bg-[#cbd4d0]">Save correction</button><button type="button" onClick={() => setCorrecting(false)} className="text-[14px] text-[#606762]">Cancel</button></div></form> : null}
    {evidenceOpen ? <FindingEvidence findingId={finding.findingId} /> : null}
  </article>;
}

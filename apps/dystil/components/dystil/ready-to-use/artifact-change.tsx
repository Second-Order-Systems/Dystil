"use client";

import { useRef, useState } from "react";
import type { ArtifactChangePreview, ReadyArtifactDetail } from "@/lib/utils/tauri";

type Props = { detail: ReadyArtifactDetail; preview: ArtifactChangePreview | null; pending: boolean; onPropose: (request: string) => Promise<boolean>; onRetry: () => void; onConfirm: () => void; onReject: () => void };

export function ArtifactChange({ detail, preview, pending, onPropose, onRetry, onConfirm, onReject }: Props) {
  const [editing, setEditing] = useState(false);
  const [request, setRequest] = useState("");
  const input = useRef<HTMLTextAreaElement>(null);

  if (preview) return <section aria-label="Proposed artifact change" className="mt-6 rounded-[12px] border border-[#bdd9ce] bg-[#f5faf7] p-5">
    <h4 className="text-[16px] font-medium text-[#24302b]">How it would read</h4>
    <p className="mt-1 text-[13px] text-[#68716c]">{preview.changedLineCount} {preview.changedLineCount === 1 ? "line differs" : "lines differ"}. Nothing changes until you confirm.</p>
    {preview.title !== detail.card.title ? <p className="mt-4 text-[15px]"><span className="line-through text-[#8a5c55]">{detail.card.title}</span><br /><span className="font-medium text-[#185f4b]">{preview.title}</span></p> : null}
    <pre className="mt-4 max-h-80 overflow-auto whitespace-pre-wrap rounded-[9px] bg-white p-4 text-[13px] leading-6 text-[#303633]">{preview.body.split("\n").map((line, index) => <span key={`${index}-${line}`} className="block"><mark className={detail.body.split("\n")[index] === line ? "bg-transparent text-inherit" : "rounded-[3px] bg-[#d9f3e7] px-0.5 text-inherit"}>{line || " "}</mark></span>)}</pre>
    <div className="mt-4 flex flex-wrap gap-4"><button disabled={pending} onClick={onConfirm} className="rounded-[9px] bg-[#087b5f] px-4 py-2 text-[14px] font-medium text-white disabled:bg-[#cbd4d0]">Keep this version</button><button disabled={pending} onClick={onRetry} className="text-[14px] font-medium text-[#0f6e56]">Try again</button><button disabled={pending} onClick={onReject} className="text-[14px] text-[#626965]">Leave it as it was</button></div>
  </section>;

  if (!editing) return <button onClick={() => { setEditing(true); queueMicrotask(() => input.current?.focus()); }} className="mt-6 text-[14px] font-medium text-[#0f6e56]">Ask Dystil to change it</button>;

  return <form className="mt-6 rounded-[12px] border border-[#d8ded9] p-4" onSubmit={async (event) => { event.preventDefault(); if (await onPropose(request)) setEditing(false); }}><label htmlFor={`change-${detail.card.artifactId}`} className="text-[16px] font-medium text-[#2b312e]">What should it do differently?</label><p className="mt-1 text-[13px] leading-5 text-[#68706c]">Say it however you would say it out loud. Dystil rewrites it and shows you before anything changes.</p><textarea ref={input} required maxLength={2000} id={`change-${detail.card.artifactId}`} value={request} onChange={(event) => setRequest(event.target.value)} placeholder="Stop putting a summary at the top, I always delete it" className="mt-3 min-h-24 w-full resize-y rounded-[9px] border border-[#cfcec8] bg-white p-3 text-[14px] leading-6 outline-none focus:border-[#087b5f]" /><div className="mt-3 flex gap-4"><button disabled={pending} className="rounded-[9px] bg-[#087b5f] px-4 py-2 text-[14px] font-medium text-white disabled:bg-[#cbd4d0]">Preview change</button><button type="button" onClick={() => setEditing(false)} className="text-[14px] text-[#626965]">Cancel</button></div></form>;
}

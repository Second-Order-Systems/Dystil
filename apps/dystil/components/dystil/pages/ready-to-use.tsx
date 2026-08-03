"use client";

import { useEffect, useRef } from "react";
import { PageHeading } from "../page-primitives";
import { ArtifactCard } from "../ready-to-use/artifact-card";
import { useReadyArtifacts } from "../ready-to-use/use-ready-artifacts";

export function ReadyToUse({ onAsk, onWorthFixing }: { onAsk: () => void; onWorthFixing: () => void }) {
  const model = useReadyArtifacts();
  const page = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (model.notice !== "Artifact removed.") return;
    queueMicrotask(() => (page.current?.querySelector("article") ?? page.current?.querySelector("h1"))?.focus());
  }, [model.notice]);

  if (model.loading && !model.items.length) return <div className="mx-auto max-w-[1124px]" aria-busy="true"><PageHeading title="Ready to use" description="Loading the things you kept…" /></div>;

  return <div ref={page} className="mx-auto max-w-[1124px]">
    <PageHeading title={model.items.length ? `${model.items.length} ${model.items.length === 1 ? "thing" : "things"}, ready when you are.` : "Ready to use"} description={model.items.length ? "Everything you kept. Each one works from here, so nothing needs setting up again." : "Prompts, runbooks, and useful tools you keep from Worth fixing will live here. Nothing is added without your say-so."} />
    <div aria-live="polite" className="mt-5 min-h-6 text-[14px] text-[#446158]">{model.notice}</div>
    {model.error ? <div role="alert" className="mt-3 rounded-[10px] bg-[#fff4f1] px-4 py-3 text-[14px] text-[#7d3028]">Ready to use hit a problem: {model.error} <button className="ml-2 font-medium underline" onClick={() => void model.load()}>Try again</button></div> : null}
    {!model.items.length ? <ReadyEmptyState waitingCount={model.waitingCount} onWorthFixing={onWorthFixing} onAsk={onAsk} /> : <><section className="mt-8" aria-label="Ready artifacts">{model.items.map((artifact) => <ArtifactCard key={artifact.artifactId} artifact={artifact} detail={model.detail} provenance={model.provenance} preview={model.preview} pending={model.pending === artifact.artifactId || model.pending === `change:${artifact.artifactId}`} onAction={(action) => { if (action === "copy") void model.copy(artifact); else if (action === "share") void model.share(artifact); else void model.open(artifact, action); }} onClose={model.close} onProvenance={() => void model.showProvenance(artifact.artifactId)} onPropose={(request) => model.propose(artifact.artifactId, request)} onRetry={() => void model.retry()} onConfirm={() => void model.confirm()} onReject={() => void model.reject()} onRemove={() => { if (window.confirm(`Remove “${artifact.title}”? This cannot be undone.`)) void model.remove(artifact.artifactId); }} />)}</section><button type="button" onClick={onAsk} className="mt-7 text-[15px] font-medium text-[#087b5f] hover:text-[#055d49] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-[#087b5f]">Want Dystil to look at something else? Ask for a fix →</button></>}
  </div>;
}

const READY_KINDS = [
  ["Prompt", "Reusable instructions for work you want done the same way again."],
  ["Runbook", "A clear sequence for work that still needs your judgment."],
  ["Tool you already have", "A direct path to a capability that can already help."],
] as const;

function ReadyEmptyState({ waitingCount, onWorthFixing, onAsk }: { waitingCount: number; onWorthFixing: () => void; onAsk: () => void }) {
  const worthFixingLabel = waitingCount > 0
    ? `Review ${waitingCount} ${waitingCount === 1 ? "finding" : "findings"} in Worth fixing`
    : "See Worth fixing";

  return <section aria-labelledby="ready-empty-title" className="mt-10 border-y border-[#d9d7d0]">
    <div className="grid lg:grid-cols-[minmax(280px,0.82fr)_minmax(360px,1.18fr)]">
      <div className="py-9 pr-8 lg:border-r lg:border-[#d9d7d0] lg:pr-12">
        <p className="text-[13px] font-medium text-[#157252]">Your saved toolkit</p>
        <h2 id="ready-empty-title" className="mt-3 max-w-[18ch] text-[27px] font-normal leading-[1.24] tracking-[-0.025em] text-[#151616]">Keep a finding once. Use it again from here.</h2>
        <p className="mt-4 max-w-[42ch] text-[15px] leading-6 text-[#5c625f]">When Dystil finds something worth fixing, you decide whether it belongs here.</p>
        <div className="mt-7 flex flex-wrap items-center gap-x-5 gap-y-3">
          <button type="button" onClick={onWorthFixing} className="rounded-[8px] bg-[#157252] px-4 py-2.5 text-[14px] font-medium text-white transition-colors hover:bg-[#0f5d43] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-3 focus-visible:outline-[#157252]">{worthFixingLabel}</button>
          <button type="button" onClick={onAsk} className="text-[14px] font-medium text-[#157252] underline decoration-[#a8cbbc] underline-offset-4 hover:text-[#0f5d43] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-[#157252]">Ask for a fix</button>
        </div>
      </div>
      <div className="border-t border-[#d9d7d0] py-2 lg:border-t-0 lg:pl-12">
        <p className="py-4 text-[13px] text-[#747a76]">What you can keep here</p>
        <dl>
          {READY_KINDS.map(([title, description], index) => <div key={title} className="grid grid-cols-[30px_minmax(0,1fr)] gap-3 border-t border-[#e5e3dd] py-5">
            <span aria-hidden="true" className="pt-0.5 text-[12px] tabular-nums text-[#8fa49b]">0{index + 1}</span>
            <div className="grid gap-1 sm:grid-cols-[150px_minmax(0,1fr)] sm:gap-5">
              <dt className="text-[15px] font-medium text-[#1d1f1e]">{title}</dt>
              <dd className="max-w-[46ch] text-[14px] leading-[1.55] text-[#686e6a]">{description}</dd>
            </div>
          </div>)}
        </dl>
      </div>
    </div>
  </section>;
}

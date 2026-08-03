"use client";

import { useEffect, useRef } from "react";
import { PageHeading } from "../page-primitives";
import { FindingCard } from "../worth-fixing/finding-card";
import { useWorthFixing } from "../worth-fixing/use-worth-fixing";

type Props = { onAsk: () => void; onReady: () => void; onSetup: () => void };

const FIRST_OPEN_SIGNALS = [
  ["The same work, over and over", "You do it the same way every time. If nothing about it changes, it does not need you."],
  ["Work that arrives on a schedule", "The Monday report, the month-end close. Most of that time is setup and waiting, and it can be done before you sit down."],
  ["Work where you make the call", "The judgement has to be yours. Rebuilding the same groundwork before every one of them does not."],
  ["Work that could come out better", "The report, the reply, the summary. Done to the standard you would want if you had the time."],
  ["What you would do if you had the time", "The prep before the call, the check before the decision. Skipped because the day is full, not because it does not matter."],
] as const;

export function WorthFixing({ onAsk, onReady, onSetup }: Props) {
  const model = useWorthFixing();
  const page = useRef<HTMLDivElement>(null);
  const selected = model.summary?.selected ?? [];

  useEffect(() => {
    if (!model.notice) return;
    queueMicrotask(() => (selected.length ? page.current?.querySelector("article") : page.current?.querySelector("h1"))?.focus());
  }, [model.notice, selected.length]);

  if (!model.summary && model.loading) return <div className="mx-auto max-w-[1124px]" aria-busy="true"><PageHeading title="Worth fixing" description="Looking at the work Dystil has already processed…" /></div>;

  if (model.summary && selected.length === 0) return <div ref={page} className="mx-auto max-w-[1124px]">
    <PageHeading title="Dystil has started reading how you work." description={<>It will let you know the moment it finds something that could save you time or make the work better.</>} />
    <div aria-live="polite" className="mt-5 min-h-6 text-[14px] text-[#446158]">{model.notice ? <>{model.notice} <button className="font-medium underline underline-offset-2" onClick={onReady}>Open in Ready to use</button></> : model.summary.processing ? "Dystil is checking recent work." : null}</div>
    {model.error ? <div role="alert" className="mt-3 rounded-[10px] bg-[#fff4f1] px-4 py-3 text-[14px] text-[#7d3028]">Worth fixing could not refresh: {model.error} <button className="ml-2 font-medium underline" onClick={() => void model.load()}>Try again</button></div> : null}
    {!model.summary.providerReady ? <section className="mt-5 rounded-[12px] border border-[#d8ded9] bg-[#f7faf8] p-5"><h2 className="text-[17px] font-medium">Connect a model to find new opportunities</h2><p className="mt-1 max-w-[70ch] text-[15px] leading-6 text-[#59615d]">Model setup controls which provider Dystil uses for future analysis.</p><button onClick={onSetup} className="mt-3 text-[14px] font-medium text-[#0f6e56] underline underline-offset-3">Open model settings</button></section> : null}
    <section className="mt-10"><h2 className="text-[22px] font-normal text-black">What it is looking for</h2><div className="mt-5 grid gap-3">{FIRST_OPEN_SIGNALS.map(([title, description]) => <article key={title} className="rounded-[14px] border border-[#deddd8] bg-white px-6 py-5"><h3 className="text-[20px] text-black">{title}</h3><p className="mt-1 max-w-[72ch] text-[16px] leading-7 text-[#505761]">{description}</p></article>)}</div></section>
    {model.summary.providerReady && model.summary.manualRefreshReady ? <button disabled={model.loading} onClick={() => void model.refresh()} className="mt-7 rounded-[10px] bg-[#087b5f] px-5 py-2.5 text-[15px] font-medium text-white disabled:bg-[#cbd4d0]">{model.loading ? "Checking…" : "Check recent work"}</button> : null}
    <AskBlock onAsk={onAsk} />
  </div>;

  return <div ref={page} className="mx-auto max-w-[1124px]">
    <PageHeading title="These are worth fixing." description="A short list of work where a prepared prompt, runbook, or existing tool could help. Review the evidence, then keep only what is useful." />
    <div aria-live="polite" className="mt-5 min-h-6 text-[14px] text-[#446158]">{model.notice ? <>{model.notice} <button className="font-medium underline underline-offset-2" onClick={onReady}>Open in Ready to use</button></> : model.summary?.processing ? "Dystil is checking recent work. Current findings remain available." : null}</div>
    {model.error ? <div role="alert" className="mt-3 rounded-[10px] bg-[#fff4f1] px-4 py-3 text-[14px] text-[#7d3028]">Worth fixing could not refresh: {model.error} <button className="ml-2 font-medium underline" onClick={() => void model.load()}>Try again</button></div> : null}
    {model.summary && !model.summary.providerReady ? <section className="mt-5 rounded-[12px] border border-[#d8ded9] bg-[#f7faf8] p-5"><h2 className="text-[17px] font-medium">Connect a model to find new opportunities</h2><p className="mt-1 max-w-[70ch] text-[15px] leading-6 text-[#59615d]">Your existing findings stay on this computer. Model setup controls which provider Dystil uses for future analysis.</p><button onClick={onSetup} className="mt-3 text-[14px] font-medium text-[#0f6e56] underline underline-offset-3">Open model settings</button></section> : null}
    <section className="mt-8" aria-label="Current Worth fixing findings">{selected.map((finding) => <FindingCard key={finding.findingId} finding={finding} pending={model.pendingId === finding.findingId} onKeep={(id) => void model.keep(id)} onDismiss={(id, reason) => void model.dismiss(id, reason)} onCorrect={model.correct} />)}</section>
    {model.summary && model.summary.eligibleCount > selected.length ? <section className="mt-7"><button className="text-[15px] font-medium text-[#0f6e56] hover:text-[#094b3b]" onClick={() => void model.loadOther()}>See the rest ({model.summary.eligibleCount - selected.length})</button>{model.other.length ? <div className="mt-4">{model.other.map((finding) => <FindingCard key={finding.findingId} finding={finding} pending={model.pendingId === finding.findingId} onKeep={(id) => void model.keep(id)} onDismiss={(id, reason) => void model.dismiss(id, reason)} onCorrect={model.correct} />)}{model.otherCursor ? <button className="mt-4 text-[14px] text-[#0f6e56]" onClick={() => void model.loadOther()}>Load more</button> : null}</div> : null}</section> : null}
    <AskBlock onAsk={onAsk} />
  </div>;
}

function AskBlock({ onAsk }: { onAsk: () => void }) {
  return <section className="mt-10 rounded-[14px] bg-[#f5fbf8] px-7 py-7"><h2 className="text-[20px] font-normal text-black">Something already annoying you?</h2><p className="mt-2 max-w-[70ch] text-[16px] leading-7 text-[#505761]">Tell Dystil what annoys you most and it can look there first.</p><button type="button" onClick={onAsk} className="mt-5 rounded-[11px] bg-[#087b5f] px-5 py-3 text-[16px] font-medium text-white hover:bg-[#06634d] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-3 focus-visible:outline-[#087b5f]">Ask for a fix</button></section>;
}

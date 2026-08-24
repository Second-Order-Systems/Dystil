"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { ArrowUp, Check, Copy, Eye, History, House, RotateCcw, Square, X } from "lucide-react";

import type {
  AskArtifact,
  AskInputEvent,
  AskPresentation,
  AskQuestion,
  AskSessionHistoryItem,
  AskSessionView,
  AskUnderstanding,
} from "@/lib/utils/tauri";
import { commands } from "@/lib/utils/tauri";
import { useAskForFix } from "@/components/dystil/ask-for-fix/use-ask-for-fix";
import { useAppPolicy } from "@/lib/app-policy";
import { Droplet } from "@/components/dystil/primitives/droplet";

const examples = [
  "Every Friday I rebuild the same client report from scattered files.",
  "I keep copying the same customer details between two apps.",
  "Our final review catches the same avoidable mistakes every time.",
];

/**
 * Dystil's turns are marked with the droplet rather than an avatar bubble —
 * the design uses the motif, not a chat-app convention. The current turn gets
 * a slightly larger mark so the eye lands on it.
 */
function TurnMark({ current = false }: { current?: boolean }) {
  return (
    <span className="flex shrink-0 justify-center pt-[6px]">
      <Droplet
        width={current ? 11 : 9}
        height={current ? 14 : 12}
        className="text-green-mark"
      />
    </span>
  );
}

function historyDate(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? "Earlier"
    : new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", year: "numeric" }).format(date);
}

function historyItemFromSession(session: AskSessionView): AskSessionHistoryItem {
  const firstUserMessage = session.messages.find((message) => message.role === "user")?.text;
  return {
    sessionId: session.sessionId,
    title: firstUserMessage || "Untitled request",
    phase: session.phase,
    status: session.status,
    createdAt: session.createdAt,
    updatedAt: session.updatedAt,
  };
}

function HistoryPanel({
  open,
  items,
  loading,
  error,
  onClose,
  onSelect,
}: {
  open: boolean;
  items: AskSessionHistoryItem[];
  loading: boolean;
  error: string | null;
  onClose: () => void;
  onSelect: (item: AskSessionHistoryItem) => void;
}) {
  if (!open) return null;
  return (
    <aside aria-label="Previous chats" className="fixed inset-y-0 right-0 z-40 flex w-full max-w-[480px] flex-col border-l border-[#d9dfda] bg-[#fbfcfa] shadow-[-18px_0_42px_rgba(31,42,34,0.13)]">
      <header className="flex items-center justify-between border-b border-[#e1e6e1] px-5 py-5">
        <div><h2 className="font-display text-[24px] font-normal tracking-[-0.025em] text-ink">Previous chats</h2><p className="mt-1 text-[13px] text-muted-ink">Read earlier conversations without changing them.</p></div>
        <button type="button" onClick={onClose} className="grid h-9 w-9 place-items-center rounded-full text-ink-3 transition-colors hover:bg-chrome hover:text-ink focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c]" aria-label="Close previous chats"><X size={18} /></button>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {loading ? <p className="px-5 py-7 text-[14px] text-muted-ink">Loading conversations…</p> : error ? <p role="alert" className="m-5 rounded-[12px] bg-[#f7eeeb] px-4 py-3 text-[13px] leading-5 text-[#753d33]">{error}</p> : items.length === 0 ? <p className="px-5 py-7 text-[14px] leading-6 text-muted-ink">Your earlier conversations will appear here.</p> : <div className="divide-y divide-[#e5e9e5]">{items.map((item) => <button key={item.sessionId} type="button" onClick={() => onSelect(item)} className="w-full px-5 py-4 text-left transition-colors hover:bg-[#f2f5f2] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[#16805c]"><p className="line-clamp-2 text-[14px] font-medium leading-5 text-ink">{item.title}</p><p className="mt-1.5 text-[12px] text-muted-ink">{historyDate(item.updatedAt)} · {item.status === "answered" ? "Complete" : "In progress"}</p></button>)}</div>}
      </div>
    </aside>
  );
}

function ConversationMessages({ session }: { session: AskSessionView }) {
  const lastAssistant = [...session.messages].reverse().find((m) => m.role !== "user");
  return (
    <div className="space-y-6">
      {session.messages.map((message) =>
        message.role === "user" ? (
          <div key={message.messageId} className="flex justify-end">
            <div className="max-w-[min(78%,620px)] whitespace-pre-wrap break-words rounded-card bg-chrome px-4 py-3 text-body-lg leading-[1.55] text-ink-2">
              {message.text}
            </div>
          </div>
        ) : (
          <div key={message.messageId} className="flex gap-3">
            <TurnMark current={message.messageId === lastAssistant?.messageId} />
            <div className="min-w-0 max-w-[70ch] whitespace-pre-wrap break-words text-pretty text-hero leading-[1.6] text-ink-2">
              {message.text}
            </div>
          </div>
        ),
      )}
    </div>
  );
}

function QuestionCard({
  question,
  questionId,
  questionNumber,
  maxQuestions,
  disabled,
  onSubmit,
  onCustom,
}: {
  question: AskQuestion;
  questionId: string | null;
  questionNumber: number;
  maxQuestions: number;
  disabled: boolean;
  onSubmit: (text: string, event: AskInputEvent) => void;
  onCustom: () => void;
}) {
  const [selected, setSelected] = useState<string[]>([]);
  useEffect(() => setSelected([]), [questionId]);
  const optionsById = useMemo(() => new Map(question.options.map((option) => [option.id, option])), [question.options]);
  const isMulti = question.kind === "multi_select";
  const minimum = Math.max(question.minSelections, 1);
  const maximum = Math.max(question.maxSelections, minimum);
  const canSubmit = selected.length >= minimum && selected.length <= maximum;

  const choose = (id: string) => {
    if (disabled) return;
    if (!isMulti) {
      setSelected([id]);
      return;
    }
    setSelected((current) => current.includes(id)
      ? current.filter((value) => value !== id)
      : current.length < maximum ? [...current, id] : current);
  };

  const submitSelection = () => {
    const chosen = selected.map((id) => optionsById.get(id)).filter(Boolean);
    if (!canSubmit || chosen.length === 0) return;
    const text = chosen.map((option) => `${option!.label}${option!.description ? ` — ${option!.description}` : ""}`).join(isMulti ? "; " : "");
    onSubmit(text, { kind: question.kind, questionId, selectedOptionIds: selected });
  };

  if (question.kind === "free_text") return null;

  return (
    <section className="ml-0 mt-5 overflow-hidden rounded-[15px] bg-[#fbfcfa] shadow-[0_15px_38px_rgba(28,42,33,0.09)] ring-1 ring-[#cfd6d1] sm:ml-[50px]" aria-label="Answer choices">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-[#e4e8e4] px-4 py-3 text-[12px]">
        <span className="font-semibold text-[#116849]">{question.kind === "compare" ? "Which reading is closer?" : isMulti ? "Select all that apply" : "Choose the closest answer"}</span>
        <span className="text-[#7c847e]">Question {questionNumber} of up to {maxQuestions}</span>
      </div>
      {question.helper && <p className="px-4 pt-4 text-[13px] leading-5 text-[#69716b]">{question.helper}</p>}
      <div className={question.kind === "compare" ? "grid gap-3 p-4 md:grid-cols-2" : "space-y-1 p-2"} role="group" aria-label={question.text}>
        {question.options.map((option, index) => {
          const active = selected.includes(option.id);
          return question.kind === "compare" ? (
            <button key={option.id} type="button" disabled={disabled} aria-pressed={active} onClick={() => choose(option.id)} className={`min-h-[158px] rounded-[12px] p-5 text-left outline-none transition-[background-color,border-color,box-shadow,transform] focus-visible:ring-2 focus-visible:ring-[#16805c] disabled:cursor-not-allowed disabled:opacity-60 ${active ? "border border-[#79aa93] bg-[#edf6f1] shadow-[0_7px_18px_rgba(33,78,56,0.08)]" : "border border-[#d8ddd9] bg-white hover:-translate-y-0.5 hover:border-[#a9bcb1]"}`}>
              <span className="text-[11px] font-semibold text-[#16805c]">Reading {index === 0 ? "A" : "B"}</span>
              <span className="mt-5 block text-[16px] font-semibold leading-5 text-[#222824]">{option.label}</span>
              <span className="mt-2 block text-[13px] leading-5 text-[#667069]">{option.description}</span>
            </button>
          ) : (
            <button key={option.id} type="button" disabled={disabled} aria-pressed={active} onClick={() => choose(option.id)} className={`grid min-h-[62px] w-full grid-cols-[28px_minmax(0,1fr)] items-center gap-3 rounded-[10px] px-3 py-2.5 text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-[#16805c] disabled:cursor-not-allowed disabled:opacity-60 ${active ? "bg-[#edf6f1]" : "hover:bg-[#f1f3f0]"}`}>
              <span className={`grid h-6 w-6 place-items-center text-[11px] font-semibold ${isMulti ? "rounded-[6px]" : "rounded-full"} ${active ? "bg-[#16805c] text-white" : "border border-[#cfd5d0] text-[#778079]"}`}>
                {active ? <Check size={14} strokeWidth={2.5} /> : String.fromCharCode(65 + index)}
              </span>
              <span className="min-w-0"><span className="block break-words text-[14px] font-medium text-[#252b27]">{option.label}</span>{option.description && <span className="mt-0.5 block break-words text-[12px] leading-5 text-[#747c76]">{option.description}</span>}</span>
            </button>
          );
        })}
      </div>
      <div className="flex flex-wrap items-center gap-3 border-t border-[#e4e8e4] px-4 py-3">
        <button type="button" onClick={onCustom} disabled={disabled} className="text-[13px] font-medium text-[#56605a] underline-offset-4 hover:text-[#116849] hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-3 focus-visible:outline-[#16805c] disabled:opacity-50">Answer in my own words</button>
        <button type="button" onClick={submitSelection} disabled={disabled || !canSubmit} className="ml-auto min-h-9 rounded-[9px] bg-[#176f51] px-4 text-[13px] font-semibold text-white shadow-[0_6px_15px_rgba(18,82,58,0.17)] transition-colors hover:bg-[#105d43] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c] disabled:cursor-not-allowed disabled:bg-[#d8ded9] disabled:text-[#88908a]">
          {isMulti ? `Use ${selected.length || "these"} answer${selected.length === 1 ? "" : "s"}` : "Use this answer"}
        </button>
      </div>
    </section>
  );
}

function UnderstandingCard({ understanding, busy, cloudAsk, onConfirm, onRefine }: { understanding: AskUnderstanding; busy: boolean; cloudAsk: boolean; onConfirm: (summary?: string) => void; onRefine: () => void }) {
  const summary = understanding.synthesis.trim() || understanding.solutionTarget;
  return (
    <section className="mt-6 rounded-[16px] bg-[#eef5f1] p-6 shadow-[0_16px_38px_rgba(27,48,35,0.10)] ring-1 ring-[#c9d9d0] sm:p-7">
      <p className="text-[12px] font-semibold text-[#116849]">Ready when you are</p>
      <h2 className="mt-2 max-w-[28ch] text-balance text-[26px] font-medium leading-[1.28] tracking-[-0.025em] text-[#202722]">{cloudAsk ? "Dystil will watch for this workflow." : "Dystil is ready to solve this workflow."}</h2>
      <p className="mt-4 max-w-[66ch] text-[15px] font-medium leading-6 text-[#303a33]">{summary}</p>
      {cloudAsk ? <p className="mt-3 max-w-[66ch] text-[14px] leading-6 text-[#59665e]">When it finds a complete, repeatable example, Dystil will come back with a Skill or agent that solves it. If it has not found one after a week, it will check in.</p> : <p className="mt-3 max-w-[66ch] text-[14px] leading-6 text-[#59665e]">Dystil will turn this into a reusable solution you can keep and use again.</p>}
      <div className="mt-6 flex flex-wrap gap-2.5"><button type="button" disabled={busy || !summary} onClick={() => onConfirm()} className="min-h-10 rounded-[9px] bg-[#176f51] px-5 text-[14px] font-semibold text-white hover:bg-[#105d43] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c] disabled:opacity-50">{cloudAsk ? "Start watching" : "Solve this"}</button><button type="button" disabled={busy} onClick={onRefine} className="min-h-10 rounded-[9px] border border-[#ccd4ce] bg-white px-4 text-[14px] font-medium text-[#4f5952] hover:bg-[#f1f4f1] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c] disabled:opacity-50">{cloudAsk ? "Keep clarifying" : "Refine this"}</button></div>
    </section>
  );
}

function ArtifactCard({ artifact, route, kept, busy, onKeep, onRevise }: { artifact: AskArtifact; route: AskPresentation["route"]; kept: boolean; busy: boolean; onKeep: () => void; onRevise: () => void }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    const content = artifact.kind === "prompt"
      ? artifact.body
      : artifact.kind === "runbook"
        ? artifact.steps.join("\n")
        : [artifact.tool, artifact.capability, ...artifact.instructions].filter(Boolean).join("\n");
    await navigator.clipboard.writeText(content);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1800);
  };
  return (
    <article className="mt-5 overflow-hidden rounded-[16px] bg-white shadow-[0_20px_50px_rgba(23,34,27,0.13)] ring-1 ring-[#c8d0ca]">
      <header className="bg-[#202622] px-5 py-5 text-white sm:px-6">
        <div className="flex flex-wrap justify-between gap-2 text-[11px] font-semibold text-[#55c696]"><span>{route.replaceAll("_", " ")} · {artifact.kind.replaceAll("_", " ")}</span><span className="font-normal text-[#a7b0aa]">Based on the current understanding</span></div>
        <h3 className="mt-3 text-[22px] font-medium tracking-[-0.025em]">{artifact.title}</h3>
        <p className="mt-1.5 max-w-[68ch] text-[13px] leading-5 text-[#b7c0ba]">{artifact.description}</p>
      </header>
      <div className="px-5 py-4 sm:px-6">
        {artifact.kind === "prompt" && <pre className="max-h-[360px] overflow-auto whitespace-pre-wrap break-words rounded-[10px] bg-[#eff1ee] p-4 font-mono text-[13px] leading-6 text-[#343b36]">{artifact.body}</pre>}
        {artifact.kind === "runbook" && <ol>{artifact.steps.map((step, index) => <li key={`${index}-${step}`} className="grid grid-cols-[30px_minmax(0,1fr)] gap-3 border-b border-[#e5e8e5] py-4 last:border-0"><span className="pt-0.5 text-[11px] font-semibold text-[#88918a]">{String(index + 1).padStart(2, "0")}</span><span className="break-words text-[14px] leading-6 text-[#343b36]">{step}</span></li>)}</ol>}
        {artifact.kind === "existing_capability" && <div className="py-2"><p className="text-[12px] font-semibold text-[#16805c]">{artifact.tool}</p><h4 className="mt-1 text-[18px] font-semibold text-[#252b27]">{artifact.capability}</h4><ol className="mt-4 space-y-3">{artifact.instructions.map((instruction, index) => <li key={`${index}-${instruction}`} className="flex gap-3 text-[14px] leading-6 text-[#505952]"><span className="font-semibold text-[#16805c]">{index + 1}</span><span>{instruction}</span></li>)}</ol></div>}
      </div>
      <footer className="flex flex-wrap gap-2 border-t border-[#e3e7e3] px-5 py-4 sm:px-6">
        <button type="button" onClick={() => void copy()} className="inline-flex min-h-9 items-center gap-2 rounded-[9px] bg-[#176f51] px-4 text-[13px] font-semibold text-white hover:bg-[#105d43] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c]"><Copy size={14} />{copied ? "Copied" : artifact.kind === "prompt" ? "Copy prompt" : artifact.kind === "runbook" ? "Copy runbook" : "Copy instructions"}</button>
        <button type="button" disabled={busy || kept} onClick={onKeep} className="min-h-9 rounded-[9px] border border-[#cbd3cd] px-4 text-[13px] font-medium text-[#4e5851] hover:bg-[#f0f3f0] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c] disabled:cursor-default disabled:bg-[#f1f3f1] disabled:text-[#7b847e]">{kept ? "Kept in Ready to use" : "Keep in Ready to use"}</button>
        <button type="button" disabled={busy} onClick={onRevise} className="min-h-9 px-2 text-[13px] font-medium text-[#56605a] underline-offset-4 hover:text-[#116849] hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c] disabled:opacity-50">Ask Dystil to change it</button>
      </footer>
    </article>
  );
}

function WatchCard({ session, busy, onStart, onStop, onReview, onGuidance }: { session: AskSessionView; busy: boolean; onStart: () => void; onStop: () => void; onReview: () => void; onGuidance: (guidance: string) => void }) {
  const watch = session.watch;
  const [guidance, setGuidance] = useState("");
  const [guidanceMode, setGuidanceMode] = useState(false);
  const canOfferWatch = session.presentation?.route === "cannot_see" || session.presentation?.route === "something_now_more_later";
  if (!watch && !canOfferWatch) return null;
  if (watch?.state === "stopped" || watch?.state === "dismissed") return null;
  return (
    <aside className="mt-5 rounded-[14px] border border-[#e4d4ab] bg-[#fff9ed] p-5">
      <div className="flex gap-3"><span className="mt-0.5 grid h-8 w-8 shrink-0 place-items-center rounded-full bg-[#f5e4b8] text-[#8c6615]"><Eye size={16} /></span><div className="min-w-0"><p className="text-[11px] font-semibold text-[#8c6615]">{watch?.state === "review_ready" ? "Evidence ready" : watch ? "Keep watching" : "Not enough evidence yet"}</p><h3 className="mt-1 text-[17px] font-semibold tracking-[-0.015em] text-[#3b3425]">{watch?.state === "review_ready" ? "Dystil found a credible example" : watch ? "Dystil is watching for this work" : "Let Dystil wait for a clearer example"}</h3><p className="mt-2 max-w-[66ch] text-[13px] leading-5 text-[#685f4e]">{watch?.state === "review_ready" ? "Dystil has enough local evidence to revisit the understanding with you. It will not make a fix until you review that reading." : watch ? `Watching for: ${watch.spec.goal} ${watch.supportingEvidenceCount ? `· ${watch.supportingEvidenceCount} supporting moment${watch.supportingEvidenceCount === 1 ? "" : "s"} found` : "· No supporting moments yet"}` : "The activity found so far is not enough to trust a fix. Keep watching stays local and brings this back when there is a credible end-to-end example."}</p>{watch && watch.state !== "review_ready" && <p className="mt-2 text-[12px] leading-5 text-[#837661]">Still needed: {watch.spec.missingEvidence.length ? watch.spec.missingEvidence.join(" · ") : "a credible end-to-end instance"}</p>}{watch?.weekCheckpointDue && <div className="mt-3 rounded-[9px] bg-[#f9edcf] p-3"><p className="text-[12px] leading-5 text-[#6f5928]">It has been a week without a reliable example. Add a cue to narrow the watch, keep waiting, or stop it.</p>{guidanceMode ? <div className="mt-2 flex flex-wrap gap-2"><input value={guidance} maxLength={500} onChange={(event) => setGuidance(event.target.value)} placeholder="For example: the final handoff happens in Linear" className="min-h-9 min-w-[230px] flex-1 rounded-[7px] border border-[#d7c99f] bg-white px-3 text-[13px] text-[#3b3425] outline-none focus:border-[#9b741b]" /><button type="button" disabled={busy || !guidance.trim()} onClick={() => { onGuidance(guidance); setGuidance(""); setGuidanceMode(false); }} className="min-h-9 rounded-[7px] bg-[#9b741b] px-3 text-[13px] font-semibold text-white disabled:opacity-50">Save guidance</button></div> : <button type="button" disabled={busy} onClick={() => setGuidanceMode(true)} className="mt-2 text-[12px] font-semibold text-[#72591f] underline underline-offset-2">Give more guidance</button>}</div>}<div className="mt-4 flex flex-wrap gap-2">{watch?.state === "review_ready" ? <><button type="button" disabled={busy} onClick={onReview} className="min-h-9 rounded-[9px] bg-[#9b741b] px-4 text-[13px] font-semibold text-white hover:bg-[#805d13] disabled:opacity-50">Review what Dystil found</button><button type="button" disabled={busy} onClick={onStop} className="min-h-9 rounded-[9px] border border-[#d7c99f] bg-white px-3.5 text-[13px] font-medium text-[#705b27] hover:bg-[#fffdf7] disabled:opacity-50">Stop watching</button></> : watch ? <button type="button" disabled={busy} onClick={onStop} className="min-h-9 rounded-[9px] border border-[#d7c99f] bg-white px-3.5 text-[13px] font-medium text-[#705b27] hover:bg-[#fffdf7] disabled:opacity-50">Stop watching</button> : <button type="button" disabled={busy} onClick={onStart} className="inline-flex min-h-9 items-center gap-2 rounded-[9px] bg-[#9b741b] px-4 text-[13px] font-semibold text-white hover:bg-[#805d13] disabled:opacity-50"><Eye size={14} />Keep watching</button>}</div></div></div>
    </aside>
  );
}

function PresentationCard({ presentation, session, busy, cloudAsk, onKeep, onRevise, onStartWatching, onStopWatching, onReviewWatch, onWatchGuidance }: { presentation: AskPresentation; session: AskSessionView; busy: boolean; cloudAsk: boolean; onKeep: () => void; onRevise: () => void; onStartWatching: () => void; onStopWatching: () => void; onReviewWatch: () => void; onWatchGuidance: (guidance: string) => void }) {
  return <section className="mt-6 sm:ml-[50px]"><div className="rounded-[14px] bg-[#eef5f1] p-5 ring-1 ring-[#c9d9d0]"><p className="text-[11px] font-semibold text-[#116849]">{cloudAsk ? "Watching for a fix" : "Answer · based on the current understanding"}</p><h2 className="mt-2 text-balance text-[23px] font-medium leading-[1.35] tracking-[-0.025em] text-[#202722]">{presentation.headline}</h2><p className="mt-3 max-w-[70ch] whitespace-pre-wrap text-[14px] leading-6 text-[#505a53]">{presentation.explanation}</p>{presentation.limitations.length > 0 && <div className="mt-4 border-t border-[#d5e1da] pt-4"><p className="text-[11px] font-semibold text-[#657169]">What this does not assume</p><ul className="mt-2 space-y-1.5 text-[13px] leading-5 text-[#5c655f]">{presentation.limitations.map((item) => <li key={item}>• {item}</li>)}</ul></div>}</div><WatchCard session={session} busy={busy} onStart={onStartWatching} onStop={onStopWatching} onReview={onReviewWatch} onGuidance={onWatchGuidance} />{presentation.artifact && <ArtifactCard artifact={presentation.artifact} route={presentation.route} kept={Boolean(session.artifactKeptId)} busy={busy} onKeep={onKeep} onRevise={onRevise} />}</section>;
}

function Composer({ value, onChange, onSubmit, disabled, placeholder, autoFocus = false }: { value: string; onChange: (value: string) => void; onSubmit: () => void; disabled: boolean; placeholder: string; autoFocus?: boolean }) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => { if (autoFocus) ref.current?.focus(); }, [autoFocus]);
  useEffect(() => {
    const textarea = ref.current;
    if (!textarea) return;
    textarea.style.height = "0px";
    textarea.style.height = `${Math.min(Math.max(textarea.scrollHeight, 32), 176)}px`;
  }, [value]);
  return (
    <div className="flex min-h-[56px] items-center gap-2 overflow-hidden rounded-[30px] bg-white px-2.5 py-2 shadow-[0_10px_30px_rgba(27,39,31,0.10)] ring-1 ring-[#c9d1cb] transition-[box-shadow] focus-within:shadow-[0_14px_34px_rgba(24,62,43,0.13)] focus-within:ring-[#78a28e]">
      <div className="min-w-0 flex-1">
        <textarea ref={ref} value={value} disabled={disabled} maxLength={1600} rows={1} onChange={(event) => onChange(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) { event.preventDefault(); onSubmit(); } }} placeholder={placeholder} className="block min-h-8 max-h-44 w-full resize-none overflow-y-auto bg-transparent px-3 text-[15px] leading-8 text-[#222824] outline-none placeholder:text-[#747d76] disabled:cursor-not-allowed disabled:opacity-60" />
      </div>
      <button type="button" aria-label="Send answer" disabled={disabled || !value.trim()} onClick={onSubmit} className="grid h-10 w-10 shrink-0 place-items-center rounded-full bg-[#176f51] text-white shadow-[0_4px_12px_rgba(18,82,58,0.20)] transition-[background-color,transform] hover:bg-[#105d43] active:scale-95 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c] disabled:cursor-not-allowed disabled:bg-[#d6ddd8] disabled:text-[#87918a] disabled:shadow-none"><ArrowUp size={18} strokeWidth={2.4} /></button>
    </div>
  );
}

export function AskForFix({ initialText = "", fresh = false, sessionId, readOnly = false }: { initialText?: string; fresh?: boolean; sessionId?: string; readOnly?: boolean }) {
  const { session, loading, busy, error, optimisticText, submit, confirm, retry, cancel, keepArtifact, startWatching, stopWatching, reviewWatch, updateWatchGuidance } = useAskForFix({ fresh, sessionId });
  const router = useRouter();
  const [draft, setDraft] = useState("");
  const [customAnswer, setCustomAnswer] = useState(false);
  const [revisionMode, setRevisionMode] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [historyItems, setHistoryItems] = useState<AskSessionHistoryItem[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const { policy } = useAppPolicy();
  const cloudAsk = policy?.askBackend === "cloud";
  const initialApplied = useRef(false);
  const viewedSession = session;
  const viewingHistory = readOnly;
  const question = viewingHistory ? null : session?.currentQuestion ?? null;
  const isConsolidating = !viewingHistory && session?.phase === "consolidate" && !session.locked;
  const isAnswered = !viewingHistory && session?.status === "answered" && Boolean(session.presentation);
  const isInitialPrompt = !loading && !viewedSession?.messages.length;
  const showComposer = !viewingHistory && (revisionMode || (!isAnswered && (!isConsolidating || customAnswer) && (question?.kind === "free_text" || customAnswer || !question)));


  useEffect(() => { setCustomAnswer(false); setRevisionMode(false); setDraft(""); }, [session?.sessionId, session?.currentQuestionId, session?.phase]);
  useEffect(() => {
    if (!initialText.trim() || initialApplied.current || loading || session?.messages.length) return;
    initialApplied.current = true;
    setDraft(initialText.trim().slice(0, 1600));
  }, [initialText, loading, session?.messages.length]);
  useEffect(() => {
    if (fresh && !busy && session?.sessionId && session.messages.length > 0) {
      router.replace(`/home/chat?session=${encodeURIComponent(session.sessionId)}`);
    }
  }, [busy, fresh, router, session?.messages.length, session?.sessionId]);

  const sendDraft = () => {
    if (!draft.trim()) return;
    const event: AskInputEvent = {
      kind: isAnswered ? "revise" : session?.messages.length ? (isConsolidating ? "refine" : "free_text") : "initial_problem",
      questionId: isAnswered ? null : session?.currentQuestionId ?? null,
      selectedOptionIds: [],
    };
    const text = draft;
    setDraft("");
    setRevisionMode(false);
    void submit(text, event);
  };

  const openHistory = () => {
    setHistoryOpen(true);
    setHistoryLoading(true);
    setHistoryError(null);
    void commands.askForFixList().then((result) => {
      if (result.status === "error") throw new Error(result.error);
      const current = session ? historyItemFromSession(session) : null;
      setHistoryItems(current && !result.data.some((item) => item.sessionId === current.sessionId) ? [current, ...result.data] : result.data);
    }).catch((failure) => {
      if (session) {
        setHistoryItems([historyItemFromSession(session)]);
        setHistoryError(null);
        return;
      }
      setHistoryError(failure instanceof Error ? failure.message : "Dystil could not load previous conversations.");
    }).finally(() => setHistoryLoading(false));
  };

  const selectHistory = (item: AskSessionHistoryItem) => {
    setHistoryOpen(false);
    router.push(`/home/chat?session=${encodeURIComponent(item.sessionId)}&view=1`);
  };

  return (
    <div className="mx-auto flex min-h-[calc(100vh-112px)] max-w-[900px] flex-col pb-8">
      <header className="flex flex-wrap items-start justify-between gap-5 pb-5">
        {isInitialPrompt ? <div className="flex gap-3"><TurnMark current /><div><h1 className="max-w-[650px] text-pretty font-display text-display-sm font-normal text-ink">What problem keeps stealing your attention?</h1><p className="mt-2 max-w-[64ch] text-body-lg text-muted-ink">It can be repetitive work, a slow handoff, a confusing process, or something you cannot quite name yet. Start messy.</p></div></div> : <div />}
        <button type="button" onClick={openHistory} className="inline-flex min-h-9 items-center gap-2 rounded-[9px] px-3 text-[13px] font-medium text-ink-3 transition-colors hover:bg-chrome hover:text-ink focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c]"><History size={15} />Previous chats</button>
      </header>

      <div className={`${isInitialPrompt ? "" : "flex-1 py-7"}`} aria-live="polite">
        {loading ? <div className="flex gap-3"><TurnMark /><div className="space-y-2 pt-1"><div className="h-4 w-24 animate-pulse rounded bg-line-2" /><div className="h-5 max-w-md animate-pulse rounded bg-line-3" /></div></div> : viewedSession?.messages.length ? <ConversationMessages session={viewedSession} /> : null}

        {!viewingHistory && optimisticText && <div className="mt-7 flex justify-end"><div className="max-w-[min(78%,620px)] rounded-[16px_16px_4px_16px] bg-[#252b27] px-4 py-3 text-[15px] leading-6 text-white">{optimisticText}</div></div>}

        {!viewingHistory && busy && <div className="mt-7 flex gap-3"><TurnMark current /><div><div className="inline-flex items-center gap-2 rounded-tile bg-recessed px-3 py-2 text-ui text-muted-ink"><span className="dystil-thinking-dots" aria-hidden="true"><i /><i /><i /></span><span>{cloudAsk ? "Clarifying your request..." : session?.phase === "present" || session?.locked ? "Building the answer" : "Looking through relevant work..."}</span></div><button type="button" onClick={() => void cancel()} className="ml-3 inline-flex items-center gap-1.5 text-ui-sm font-semibold text-marigold-text hover:underline"><Square size={10} fill="currentColor" />Stop</button></div></div>}

        {!busy && question && <QuestionCard question={question} questionId={session?.currentQuestionId ?? null} questionNumber={session?.questionCount ?? 1} maxQuestions={session?.maxQuestions ?? 12} disabled={busy} onCustom={() => setCustomAnswer(true)} onSubmit={(text, event) => void submit(text, event)} />}
        {!busy && isConsolidating && session && <UnderstandingCard understanding={session.understanding} busy={busy} cloudAsk={cloudAsk} onConfirm={(summary) => void confirm(summary)} onRefine={() => setCustomAnswer(true)} />}
        {!viewingHistory && !busy && session?.presentation && <PresentationCard presentation={session.presentation} session={session} busy={busy} cloudAsk={cloudAsk} onKeep={() => void keepArtifact()} onRevise={() => setRevisionMode(true)} onStartWatching={() => void startWatching()} onStopWatching={() => void stopWatching()} onReviewWatch={() => void reviewWatch()} onWatchGuidance={(guidance) => void updateWatchGuidance(guidance)} />}

        {!viewingHistory && error && <div role="alert" className="mt-6 flex flex-wrap items-center gap-3 rounded-[12px] bg-[#f7eeeb] px-4 py-3 text-[13px] leading-5 text-[#753d33] ring-1 ring-[#e3c4bc]"><span className="min-w-0 flex-1">{error}</span>{session && <button type="button" disabled={busy} onClick={() => void retry()} className="min-h-8 rounded-[8px] bg-white px-3 font-semibold text-[#753d33] ring-1 ring-[#d8b6ad] hover:bg-[#fffaf8] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#8b3f32]">Try again</button>}</div>}
      </div>

      {!loading && !busy && showComposer && <div className={`sticky bottom-0 z-10 w-full max-w-[620px] pt-2 pb-2 ${isInitialPrompt ? "" : "mx-auto"}`}><Composer value={draft} onChange={setDraft} onSubmit={sendDraft} disabled={busy} autoFocus={customAnswer || revisionMode} placeholder={revisionMode ? "What should Dystil change in this answer?" : isConsolidating ? "What did Dystil misunderstand or miss?" : question ? "Answer in your own words…" : "Describe the problem, where it happens, and why it is annoying…"} />{!session?.messages.length && <div className="mt-4 flex flex-wrap gap-2">{examples.map((example) => <button key={example} type="button" onClick={() => setDraft(example)} className="rounded-full border border-[#d2d8d3] bg-[#fbfcfa] px-3 py-1.5 text-[12px] text-[#616a64] hover:border-[#9fb9aa] hover:text-[#116849] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c]">{example}</button>)}</div>}</div>}
      {isAnswered && <div className="flex flex-wrap justify-center gap-2 border-t border-[#e0e4e0] pt-5"><button type="button" onClick={() => router.push("/home")} className="inline-flex min-h-9 items-center gap-2 rounded-[9px] px-3 text-[13px] font-medium text-[#4e5851] hover:bg-[#f0f3f0] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c]"><House size={14} />Back to Home</button><button type="button" disabled={busy} onClick={() => router.push(cloudAsk ? "/home" : "/home/chat")} className="inline-flex min-h-9 items-center gap-2 rounded-[9px] border border-[#cbd3cd] bg-white px-4 text-[13px] font-medium text-[#4e5851] hover:bg-[#f0f3f0] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c]"><RotateCcw size={14} />Ask about another problem</button></div>}
      <HistoryPanel open={historyOpen} items={historyItems} loading={historyLoading} error={historyError} onClose={() => setHistoryOpen(false)} onSelect={selectHistory} />
    </div>
  );
}

"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowUp, Check, Copy, RotateCcw, Square } from "lucide-react";

import type {
  AskArtifact,
  AskInputEvent,
  AskPresentation,
  AskQuestion,
  AskSessionView,
  AskUnderstanding,
} from "@/lib/utils/tauri";
import { useAskForFix } from "@/components/dystil/ask-for-fix/use-ask-for-fix";

const examples = [
  "Every Friday I rebuild the same client report from scattered files.",
  "I keep copying the same customer details between two apps.",
  "Our final review catches the same avoidable mistakes every time.",
];

const phaseNames = ["Understand", "Follow up", "Consolidate", "Present"] as const;

function DystilAvatar() {
  return (
    <div className="grid h-9 w-9 shrink-0 place-items-center rounded-[11px] bg-[#1d2420] text-[13px] font-semibold text-white shadow-[0_7px_18px_rgba(26,38,30,0.16)]" aria-hidden="true">
      D
    </div>
  );
}

function PhaseRail({ session }: { session: AskSessionView | null }) {
  const current = session ? phaseNames.findIndex((name) => name.toLowerCase().replace(" ", "_") === session.phase) : 0;
  return (
    <ol className="flex items-center gap-2" aria-label="Conversation progress">
      {phaseNames.map((name, index) => (
        <li key={name} className="flex items-center gap-2">
          <span className={`h-1.5 w-8 rounded-full transition-colors ${index <= current ? "bg-[#16805c]" : "bg-[#d9ded9]"}`} />
          <span className="sr-only">{name}{index === current ? ", current phase" : ""}</span>
        </li>
      ))}
    </ol>
  );
}

function ConversationMessages({ session }: { session: AskSessionView }) {
  return (
    <div className="space-y-7">
      {session.messages.map((message) => message.role === "user" ? (
        <div key={message.messageId} className="flex justify-end">
          <div className="max-w-[min(78%,620px)] whitespace-pre-wrap break-words rounded-[16px_16px_4px_16px] bg-[#252b27] px-4 py-3 text-[15px] leading-6 text-[#f5f7f5] shadow-[0_8px_22px_rgba(24,31,27,0.12)]">
            {message.text}
          </div>
        </div>
      ) : (
        <div key={message.messageId} className="grid grid-cols-[36px_minmax(0,1fr)] gap-3.5">
          <DystilAvatar />
          <div className="min-w-0 pt-1">
            <p className="mb-1.5 text-[12px] font-medium text-[#78817b]">Dystil</p>
            <div className="max-w-[70ch] whitespace-pre-wrap break-words text-[16px] leading-[1.65] text-[#252b27]">{message.text}</div>
          </div>
        </div>
      ))}
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

function UnderstandingCard({ understanding, busy, onConfirm, onRefine }: { understanding: AskUnderstanding; busy: boolean; onConfirm: () => void; onRefine: () => void }) {
  const fields = [
    ["Grounded in", understanding.grounding.length ? understanding.grounding.join(" · ") : "Your description so far"],
    ["What I infer", understanding.inferences.length ? understanding.inferences.join(" · ") : "No material inference"],
    ["Preserve", understanding.preservedBoundary],
    ["Solution target", understanding.solutionTarget],
  ];
  return (
    <section className="mt-6 overflow-hidden rounded-[16px] bg-[#fbfdfb] shadow-[0_18px_46px_rgba(27,48,35,0.10)] ring-1 ring-[#a9c9b9]">
      <div className="flex flex-wrap items-center justify-between gap-2 bg-[#f0f6f2] px-5 py-3 text-[12px]"><span className="font-semibold text-[#116849]">Dystil&apos;s working understanding</span><span className="text-[#738078]">Synthesized from this conversation</span></div>
      <div className="p-5 sm:p-6">
        <p className="text-[11px] font-semibold text-[#647269]">My read</p>
        <h2 className="mt-2 max-w-[66ch] text-balance text-[22px] font-medium leading-[1.38] tracking-[-0.025em] text-[#1f2722]">{understanding.synthesis}</h2>
        <div className="mt-5 grid border-y border-[#dfe6e1] sm:grid-cols-2">
          {fields.map(([label, value], index) => <div key={label} className={`min-w-0 py-4 ${index % 2 === 1 ? "sm:border-l sm:border-[#dfe6e1] sm:pl-5" : "sm:pr-5"} ${index > 1 ? "border-t border-[#dfe6e1]" : index === 1 ? "border-t border-[#dfe6e1] sm:border-t-0" : ""}`}><p className="text-[11px] text-[#7b857e]">{label}</p><p className="mt-1.5 break-words text-[13px] font-medium leading-5 text-[#313934]">{value || "Not yet specified"}</p></div>)}
        </div>
        <div className="mt-4 rounded-[10px] bg-[#eef0ed] px-4 py-3 text-[13px] leading-5 text-[#5f6761]"><span className="font-semibold text-[#3f4742]">Still uncertain: </span>{understanding.uncertainty.length ? understanding.uncertainty.join(" · ") : "Nothing material."}</div>
        <div className="mt-5 flex flex-wrap gap-2.5"><button type="button" disabled={busy} onClick={onConfirm} className="min-h-10 rounded-[9px] bg-[#176f51] px-5 text-[14px] font-semibold text-white hover:bg-[#105d43] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c] disabled:opacity-50">Solve this</button><button type="button" disabled={busy} onClick={onRefine} className="min-h-10 rounded-[9px] border border-[#ccd4ce] bg-white px-4 text-[14px] font-medium text-[#4f5952] hover:bg-[#f1f4f1] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c] disabled:opacity-50">Refine the understanding</button></div>
      </div>
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

function PresentationCard({ presentation, session, busy, onKeep, onRevise }: { presentation: AskPresentation; session: AskSessionView; busy: boolean; onKeep: () => void; onRevise: () => void }) {
  return <section className="mt-6 sm:ml-[50px]"><div className="rounded-[14px] bg-[#eef5f1] p-5 ring-1 ring-[#c9d9d0]"><p className="text-[11px] font-semibold text-[#116849]">Answer · based on the current understanding</p><h2 className="mt-2 text-balance text-[23px] font-medium leading-[1.35] tracking-[-0.025em] text-[#202722]">{presentation.headline}</h2><p className="mt-3 max-w-[70ch] whitespace-pre-wrap text-[14px] leading-6 text-[#505a53]">{presentation.explanation}</p>{presentation.limitations.length > 0 && <div className="mt-4 border-t border-[#d5e1da] pt-4"><p className="text-[11px] font-semibold text-[#657169]">What this does not assume</p><ul className="mt-2 space-y-1.5 text-[13px] leading-5 text-[#5c655f]">{presentation.limitations.map((item) => <li key={item}>• {item}</li>)}</ul></div>}</div>{presentation.artifact && <ArtifactCard artifact={presentation.artifact} route={presentation.route} kept={Boolean(session.artifactKeptId)} busy={busy} onKeep={onKeep} onRevise={onRevise} />}</section>;
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
    <div className="flex min-h-[74px] items-end gap-2 overflow-hidden rounded-[30px] bg-white px-2.5 py-2.5 shadow-[0_10px_30px_rgba(27,39,31,0.10)] ring-1 ring-[#c9d1cb] transition-[box-shadow] focus-within:shadow-[0_14px_34px_rgba(24,62,43,0.13)] focus-within:ring-[#78a28e]">
      <div className="min-w-0 flex-1 py-0.5">
        <textarea ref={ref} value={value} disabled={disabled} maxLength={1600} rows={1} onChange={(event) => onChange(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) { event.preventDefault(); onSubmit(); } }} placeholder={placeholder} className="block min-h-8 max-h-44 w-full resize-none overflow-y-auto bg-transparent px-3 text-[15px] leading-8 text-[#222824] outline-none placeholder:text-[#747d76] disabled:cursor-not-allowed disabled:opacity-60" />
        <div className="flex items-center gap-3 px-3 text-[11px] leading-4 text-[#737c76]"><span>Stays on this computer · {value.length}/1600</span><span className="hidden text-[#8b938d] sm:inline">⌘ Enter to send</span></div>
      </div>
      <button type="button" aria-label="Send answer" disabled={disabled || !value.trim()} onClick={onSubmit} className="grid h-10 w-10 shrink-0 place-items-center rounded-full bg-[#176f51] text-white shadow-[0_4px_12px_rgba(18,82,58,0.20)] transition-[background-color,transform] hover:bg-[#105d43] active:scale-95 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c] disabled:cursor-not-allowed disabled:bg-[#d6ddd8] disabled:text-[#87918a] disabled:shadow-none"><ArrowUp size={18} strokeWidth={2.4} /></button>
    </div>
  );
}

export function AskForFix() {
  const { session, loading, busy, error, optimisticText, submit, confirm, retry, cancel, keepArtifact, startNew } = useAskForFix();
  const [draft, setDraft] = useState("");
  const [customAnswer, setCustomAnswer] = useState(false);
  const [revisionMode, setRevisionMode] = useState(false);
  const question = session?.currentQuestion ?? null;
  const isConsolidating = session?.phase === "consolidate" && !session.locked;
  const isAnswered = session?.status === "answered" && Boolean(session.presentation);
  const showComposer = revisionMode || (!isAnswered && (!isConsolidating || customAnswer) && (question?.kind === "free_text" || customAnswer || !question));

  useEffect(() => { setCustomAnswer(false); setRevisionMode(false); setDraft(""); }, [session?.sessionId, session?.currentQuestionId, session?.phase]);

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

  return (
    <div className="mx-auto flex min-h-[calc(100vh-112px)] max-w-[900px] flex-col pb-8">
      <header className="flex flex-wrap items-start justify-between gap-5 border-b border-[#dde1dd] pb-5">
        <div><h1 className="text-balance text-[30px] font-medium leading-tight tracking-[-0.03em] text-[#171d19]">Ask for a fix</h1><p className="mt-2 max-w-[62ch] text-[14px] leading-6 text-[#69716b]">Bring any problem that annoys you. Dystil will understand it before proposing an answer.</p></div>
        <div className="flex flex-col items-end gap-3"><PhaseRail session={session} />{session && <button type="button" disabled={busy} onClick={() => void startNew()} className="inline-flex items-center gap-1.5 text-[12px] font-medium text-[#657069] hover:text-[#116849] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-3 focus-visible:outline-[#16805c] disabled:opacity-50"><RotateCcw size={13} />New problem</button>}</div>
      </header>

      <div className="flex-1 py-7" aria-live="polite">
        {loading ? <div className="grid grid-cols-[36px_1fr] gap-3.5"><div className="h-9 w-9 animate-pulse rounded-[11px] bg-[#dfe4e0]" /><div className="space-y-2 pt-1"><div className="h-4 w-24 animate-pulse rounded bg-[#dfe4e0]" /><div className="h-5 max-w-md animate-pulse rounded bg-[#e7eae7]" /></div></div> : session?.messages.length ? <ConversationMessages session={session} /> : <div className="grid grid-cols-[36px_minmax(0,1fr)] gap-3.5"><DystilAvatar /><div className="pt-1"><p className="mb-2 text-[12px] font-medium text-[#78817b]">Dystil</p><h2 className="max-w-[650px] text-balance text-[25px] font-medium leading-[1.35] tracking-[-0.03em] text-[#202722]">What problem keeps stealing your attention?</h2><p className="mt-2 max-w-[64ch] text-[14px] leading-6 text-[#68716a]">It can be repetitive work, a slow handoff, a confusing process, or something you cannot quite name yet. Start messy.</p></div></div>}

        {optimisticText && <div className="mt-7 flex justify-end"><div className="max-w-[min(78%,620px)] rounded-[16px_16px_4px_16px] bg-[#252b27] px-4 py-3 text-[15px] leading-6 text-white">{optimisticText}</div></div>}

        {busy && <div className="mt-7 grid grid-cols-[36px_minmax(0,1fr)] gap-3.5"><DystilAvatar /><div className="pt-1"><div className="inline-flex items-center gap-2 rounded-[10px] bg-[#edf1ee] px-3 py-2 text-[13px] text-[#5e6861]"><span className="dystil-thinking-dots" aria-hidden="true"><i /><i /><i /></span><span>{session?.phase === "present" || session?.locked ? "Building the answer" : "Looking through relevant work..."}</span></div><button type="button" onClick={() => void cancel()} className="ml-3 inline-flex items-center gap-1.5 text-[12px] font-medium text-[#717a74] hover:text-[#8b3f32] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#8b3f32]"><Square size={10} fill="currentColor" />Stop</button></div></div>}

        {!busy && question && <QuestionCard question={question} questionId={session?.currentQuestionId ?? null} questionNumber={session?.questionCount ?? 1} maxQuestions={session?.maxQuestions ?? 12} disabled={busy} onCustom={() => setCustomAnswer(true)} onSubmit={(text, event) => void submit(text, event)} />}
        {!busy && isConsolidating && session && <UnderstandingCard understanding={session.understanding} busy={busy} onConfirm={() => void confirm()} onRefine={() => setCustomAnswer(true)} />}
        {!busy && session?.presentation && <PresentationCard presentation={session.presentation} session={session} busy={busy} onKeep={() => void keepArtifact()} onRevise={() => setRevisionMode(true)} />}

        {error && <div role="alert" className="mt-6 flex flex-wrap items-center gap-3 rounded-[12px] bg-[#f7eeeb] px-4 py-3 text-[13px] leading-5 text-[#753d33] ring-1 ring-[#e3c4bc]"><span className="min-w-0 flex-1">{error}</span>{session?.lastErrorCode && <button type="button" disabled={busy} onClick={() => void retry()} className="min-h-8 rounded-[8px] bg-white px-3 font-semibold text-[#753d33] ring-1 ring-[#d8b6ad] hover:bg-[#fffaf8] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#8b3f32]">Try again</button>}</div>}
      </div>

      {!loading && !busy && showComposer && <div className="sticky bottom-0 z-10 pt-5 pb-2"><Composer value={draft} onChange={setDraft} onSubmit={sendDraft} disabled={busy} autoFocus={customAnswer || revisionMode} placeholder={revisionMode ? "What should Dystil change in this answer?" : isConsolidating ? "What did Dystil misunderstand or miss?" : question ? "Answer in your own words…" : "Describe the problem, where it happens, and why it is annoying…"} />{!session?.messages.length && <div className="mt-4 flex flex-wrap gap-2">{examples.map((example) => <button key={example} type="button" onClick={() => setDraft(example)} className="rounded-full border border-[#d2d8d3] bg-[#fbfcfa] px-3 py-1.5 text-[12px] text-[#616a64] hover:border-[#9fb9aa] hover:text-[#116849] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c]">{example}</button>)}</div>}</div>}
      {isAnswered && <div className="flex justify-center border-t border-[#e0e4e0] pt-5"><button type="button" disabled={busy} onClick={() => void startNew()} className="inline-flex min-h-9 items-center gap-2 rounded-[9px] border border-[#cbd3cd] bg-white px-4 text-[13px] font-medium text-[#4e5851] hover:bg-[#f0f3f0] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#16805c]"><RotateCcw size={14} />Ask about another problem</button></div>}
    </div>
  );
}

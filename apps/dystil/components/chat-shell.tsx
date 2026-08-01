"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  AtSign, FolderSearch, Loader2, MessageSquarePlus,
  Pause, Play, Search, Send, Settings2, Trash2, Zap,
} from "lucide-react";
import { DystilBrand } from "@/components/dystil-brand";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";

type Peer = { userId: string; displayName: string | null; email: string; agentStatus: string };
type AgentMessage = { messageId: string; peerUserId: string; direction: string; kind: string; localStatus: string; text: string; evidence: Array<{ label: string; localDate: string }> };
type View = "ask" | "inquiry" | "inquiries" | "automations" | "settings";

export type ChatSession = { id: string; title: string; updatedAt: string };
export type Chat = { id: string; conversationId: string; question: string; mode: "local" | "team"; answer?: string | null; status?: "pending" | "complete" | "failed"; citations?: Array<{ label: string; localDate: string }>; provider?: string | null; model?: string | null; elapsedMs?: number | null; historical?: boolean };

type Props = {
  userName: string; userEmail: string; recording: boolean; toggling: boolean; onToggleRecording: () => void;
  screenshotEnabled: boolean; onScreenshotChange: (value: boolean) => void; screenshotBusy: boolean;
  peers: Peer[]; agentMessages: AgentMessage[]; sessions: ChatSession[];
  onLoadSession: (sessionId: string) => Promise<Chat[]>; onSendLocal: (sessionId: string, question: string) => Promise<Chat>;
  onAskPeer: (peerId: string, question: string) => Promise<void>; onLogout: () => void; loggingOut: boolean; version: string;
};

export function ChatShell({ userName, userEmail, recording, toggling, onToggleRecording, screenshotEnabled, onScreenshotChange, screenshotBusy, peers, agentMessages, sessions, onLoadSession, onSendLocal, onAskPeer, onLogout, loggingOut, version }: Props) {
  const [view, setView] = useState<View>("ask");
  const [input, setInput] = useState("");
  const [mentionOpen, setMentionOpen] = useState(false);
  const [selectedPeer, setSelectedPeer] = useState<Peer | null>(null);
  const [busy, setBusy] = useState(false);
  const [loadingSession, setLoadingSession] = useState(false);
  const [activeInquiryId, setActiveInquiryId] = useState(() => crypto.randomUUID());
  const [turns, setTurns] = useState<Chat[]>([]);
  const trailEnd = useRef<HTMLDivElement>(null);
  const activeInquiry = sessions.find((session) => session.id === activeInquiryId);
  const visiblePeers = useMemo(
    () => peers.filter((peer) => !input.includes("@") || `${peer.displayName} ${peer.email}`.toLowerCase().includes(input.replace(/^.*@/, "").toLowerCase())),
    [peers, input],
  );

  useEffect(() => { trailEnd.current?.scrollIntoView({ behavior: "smooth", block: "end" }); }, [turns, busy]);
  useEffect(() => {
    if (!busy && turns.length && !activeInquiry) {
      setTurns([]);
      setActiveInquiryId(crypto.randomUUID());
      setView("ask");
    }
  }, [activeInquiry, busy, turns.length]);

  const startInquiry = () => {
    setView("ask"); setInput(""); setSelectedPeer(null); setMentionOpen(false);
    setActiveInquiryId(crypto.randomUUID()); setTurns([]);
  };
  const openInquiry = async (sessionId: string) => {
    setView("inquiry"); setActiveInquiryId(sessionId); setLoadingSession(true); setTurns([]);
    try { setTurns(await onLoadSession(sessionId)); } finally { setLoadingSession(false); }
  };
  const submit = async () => {
    let question = input.trim();
    if (selectedPeer) {
      const prefix = `@${selectedPeer.displayName || selectedPeer.email.split("@")[0]}`;
      if (question.startsWith(prefix)) question = question.slice(prefix.length).trim();
    }
    if (!question || busy) return;
    setBusy(true); setView("inquiry");
    try {
      if (selectedPeer) {
        const turn: Chat = { id: crypto.randomUUID(), conversationId: activeInquiryId, question: `@ ${selectedPeer.displayName || selectedPeer.email.split("@")[0]} · ${question}`, mode: "team", status: "pending" };
        setTurns((current) => [...current, turn]);
        await onAskPeer(selectedPeer.userId, question);
      } else {
        const optimistic: Chat = { id: crypto.randomUUID(), conversationId: activeInquiryId, question, mode: "local", status: "pending" };
        setTurns((current) => [...current, optimistic]);
        try {
          const stored = await onSendLocal(activeInquiryId, question);
          setTurns((current) => current.map((turn) => turn.id === optimistic.id ? stored : turn));
        } catch (error) {
          const answer = error instanceof Error ? error.message : "Dystil could not save or answer this question.";
          setTurns((current) => current.map((turn) => turn.id === optimistic.id ? { ...turn, status: "failed", answer } : turn));
        }
      }
      setInput(""); setMentionOpen(false); setSelectedPeer(null);
    } finally { setBusy(false); }
  };
  const choosePeer = (peer: Peer) => { setSelectedPeer(peer); setInput(`@${peer.displayName || peer.email.split("@")[0]} `); setMentionOpen(false); };
  const nav = (target: View) => setView(target);

  return <main className="h-dvh min-w-[760px] overflow-hidden bg-[#f5f3ee] text-[#242522] dark:bg-[#151616] dark:text-[#f1f0eb]">
    <div className="grid h-full grid-cols-[196px_minmax(0,1fr)] grid-rows-[74px_minmax(0,1fr)]">
      <aside className="row-span-2 flex min-h-0 flex-col border-r border-black/10 bg-[#eeece5] px-3 pb-4 pt-5 dark:border-white/10 dark:bg-[#1d1f1e]">
        <DystilBrand className="justify-start px-2 pb-8" highlightY />
        <nav className="grid gap-1" aria-label="Dystil navigation">
          <NavItem active={view === "ask" || view === "inquiry"} icon={<Search />} label="Ask your work" onClick={() => view === "inquiry" && turns.length ? setView("inquiry") : nav("ask")} />
          <NavItem active={view === "inquiries"} icon={<FolderSearch />} label="Inquiries" onClick={() => nav("inquiries")} />
          <NavItem active={view === "automations"} icon={<Zap />} label="Automations" onClick={() => nav("automations")} />
        </nav>
        <p className="mb-2 mt-7 px-2 text-[10px] font-bold uppercase tracking-[.12em] text-[#74766f] dark:text-[#85867f]">Local memory</p>
        <nav className="grid gap-1" aria-label="Account navigation">
          <NavItem active={view === "settings"} icon={<Settings2 />} label="Capture settings" onClick={() => nav("settings")} />
        </nav>
        <div className="mt-auto border-t border-black/10 px-2 pt-4 dark:border-white/10">
          <p className="flex items-center gap-2 text-[11px] text-[#686a64] dark:text-[#aaa9a1]"><i className={cn("h-2 w-2 rounded-full", recording ? "bg-[#157252] dark:bg-[#56d59d]" : "bg-[#85867f]")} />{recording ? "Capturing locally" : "Capture paused"}</p>
          <div className="mt-4 flex items-center justify-between text-xs"><span className="grid h-7 w-7 place-items-center rounded-full border border-black/15 text-[10px] dark:border-white/15">{userName.slice(0, 2).toUpperCase()}</span><span className="max-w-[108px] truncate text-[#686a64] dark:text-[#aaa9a1]">{userName}</span><Settings2 className="h-3.5 w-3.5 text-[#686a64] dark:text-[#aaa9a1]" /></div>
        </div>
      </aside>

      <header className="flex items-center justify-between border-b border-black/10 px-7 dark:border-white/10">
        <span className="text-xs text-[#686a64] dark:text-[#aaa9a1]">{view === "inquiry" ? activeInquiry?.title || "Current inquiry" : "Dystil · local memory"}</span>
        {view === "inquiry" && <button type="button" className="inline-flex items-center gap-2 text-xs font-semibold text-[#157252] hover:text-[#0e513a] dark:text-[#56d59d] dark:hover:text-[#a5f1c8]" onClick={startInquiry}><MessageSquarePlus className="h-4 w-4" />New inquiry</button>}
      </header>

      <section className="min-h-0 overflow-hidden">
        {view === "ask" && <InquiryHome input={input} setInput={setInput} submit={submit} busy={busy} sessions={sessions} onOpenInquiry={openInquiry} />}
        {view === "inquiry" && <InquiryTrail turns={turns} loading={loadingSession} busy={busy} agentMessages={agentMessages} peers={peers} trailEnd={trailEnd} />}
        {view === "inquiries" && <Inquiries sessions={sessions} onOpen={openInquiry} onNew={startInquiry} />}
        {view === "automations" && <AutomationsPage />}
        {view === "settings" && <SettingsPage recording={recording} toggling={toggling} onToggle={onToggleRecording} screenshots={screenshotEnabled} onScreenshot={onScreenshotChange} screenshotBusy={screenshotBusy} userName={userName} userEmail={userEmail} onLogout={onLogout} loggingOut={loggingOut} version={version} />}
      </section>

      {view === "inquiry" && <Composer input={input} setInput={setInput} busy={busy} submit={submit} mentionOpen={mentionOpen} setMentionOpen={setMentionOpen} visiblePeers={visiblePeers} choosePeer={choosePeer} />}
    </div>
  </main>;
}

function NavItem({ active, icon, label, onClick }: { active: boolean; icon: React.ReactNode; label: string; onClick: () => void }) {
  return <button type="button" onClick={onClick} className={cn("flex items-center gap-3 rounded-lg px-3 py-2.5 text-left text-[13px] transition", active ? "bg-[#dfe8df] text-[#173c2b] shadow-[inset_2px_0_0_#157252] dark:bg-[#252a27] dark:text-[#f1f0eb] dark:shadow-[inset_2px_0_0_#56d59d]" : "text-[#686a64] hover:bg-black/[.045] hover:text-[#242522] dark:text-[#aaa9a1] dark:hover:bg-white/[.06] dark:hover:text-[#f1f0eb]")}>{icon && <span className="[&_svg]:h-4 [&_svg]:w-4">{icon}</span>}{label}</button>;
}

function InquiryHome({ input, setInput, submit, busy, sessions, onOpenInquiry }: { input: string; setInput: (value: string) => void; submit: () => Promise<void>; busy: boolean; sessions: ChatSession[]; onOpenInquiry: (sessionId: string) => Promise<void> }) {
  return <div className="grid h-full min-h-0 grid-cols-[minmax(0,1.25fr)_minmax(310px,.75fr)] overflow-auto">
    <div className="px-12 py-16">
      <div className="max-w-[700px]">
        <h1 className="max-w-[9ch] font-serif text-6xl leading-[.98] tracking-[-.04em]">Ask your work.</h1>
        <p className="mt-5 max-w-[52ch] text-sm leading-6 text-[#686a64] dark:text-[#aaa9a1]">Return to a decision, an unfinished thread, or a moment you know happened but cannot quite place. Dystil searches the work captured on this device.</p>
        <form className="mt-8 flex h-16 items-center gap-3 rounded-lg border border-[#157252]/60 bg-white px-4 shadow-[0_10px_34px_rgba(20,34,28,.08)] dark:bg-[#1d1f1e] dark:shadow-none" onSubmit={(event) => { event.preventDefault(); void submit(); }}>
          <Search className="h-5 w-5 shrink-0 text-[#157252] dark:text-[#56d59d]" />
          <input value={input} onChange={(event) => setInput(event.target.value)} placeholder="What did I work on for the release?" className="min-w-0 flex-1 bg-transparent font-serif text-xl outline-none placeholder:text-[#85867f]" />
          <button type="submit" disabled={busy || !input.trim()} className="grid h-10 w-10 place-items-center rounded-md bg-[#157252] text-white transition hover:bg-[#0e513a] disabled:opacity-35 dark:bg-[#56d59d] dark:text-[#151616] dark:hover:bg-[#a5f1c8]" aria-label="Ask your work">{busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}</button>
        </form>
        <div className="mt-3 flex gap-5 text-[11px] text-[#74766f] dark:text-[#85867f]"><span>— Searches local capture</span><span>— Shows sources</span><span>— Names uncertainty</span></div>

      </div>
    </div>
    <aside className="border-l border-black/10 bg-[#eeece5]/70 px-8 py-16 dark:border-white/10 dark:bg-[#1d1f1e]">
      <p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#157252] dark:text-[#56d59d]">Resume an inquiry</p>
      <div className="mt-4 divide-y divide-black/10 border-y border-black/10 dark:divide-white/10 dark:border-white/10">{sessions.slice(0, 3).map((session) => <button type="button" key={session.id} onClick={() => void onOpenInquiry(session.id)} className="block w-full py-4 text-left"><b className="block truncate font-serif text-lg font-normal">{session.title}</b><span className="mt-1 block text-[11px] text-[#686a64] dark:text-[#aaa9a1]">Updated {formatDate(session.updatedAt)}</span></button>)}{!sessions.length && <p className="py-5 text-sm leading-6 text-[#686a64] dark:text-[#aaa9a1]">Your related questions will stay together here. Start with a question above.</p>}</div>
      <div className="mt-12 border-t border-black/10 pt-5 dark:border-white/10"><p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#74766f] dark:text-[#85867f]">How Dystil responds</p><p className="mt-3 text-sm leading-6 text-[#686a64] dark:text-[#aaa9a1]"><b className="text-[#242522] dark:text-[#f1f0eb]">Grounded when possible.</b> When local reasoning is not reliable enough, Dystil shows the available capture instead of manufacturing a finding.</p></div>
    </aside>
  </div>;
}

function InquiryTrail({ turns, loading, busy, agentMessages, peers, trailEnd }: { turns: Chat[]; loading: boolean; busy: boolean; agentMessages: AgentMessage[]; peers: Peer[]; trailEnd: React.RefObject<HTMLDivElement> }) {
  return <div className="h-[calc(100%-78px)] overflow-auto px-12 py-12"><div className="mx-auto max-w-[880px]">
    <p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#157252] dark:text-[#56d59d]">Inquiry trail</p>
    <h1 className="mt-3 font-serif text-4xl tracking-[-.035em]">{turns[0]?.question || "Current inquiry"}</h1>
    <p className="mt-3 text-sm text-[#686a64] dark:text-[#aaa9a1]">Follow-up questions keep their context and available evidence together.</p>
    {loading && <p className="mt-12 text-sm text-[#686a64] dark:text-[#aaa9a1]"><Loader2 className="mr-2 inline h-4 w-4 animate-spin" />Opening inquiry…</p>}
    <div className="mt-10 divide-y divide-black/10 border-y border-black/10 dark:divide-white/10 dark:border-white/10">{turns.map((turn, index) => <article key={turn.id} className="py-8"><p className="text-[10px] font-bold uppercase tracking-[.1em] text-[#74766f] dark:text-[#85867f]">{index === 0 ? "Starting question" : "Follow-up"}</p><h2 className="mt-2 font-serif text-2xl font-normal tracking-[-.02em]">{turn.question}</h2><div className="mt-5">{turn.mode === "team" ? <TeamAnswer messages={agentMessages} peers={peers} /> : <LocalAnswer chat={turn} />}</div></article>)}</div>
    {busy && <p className="mt-5 text-xs text-[#686a64] dark:text-[#aaa9a1]">Keeping this inquiry together…</p>}<div ref={trailEnd} />
  </div></div>;
}

function Inquiries({ sessions, onOpen, onNew }: { sessions: ChatSession[]; onOpen: (id: string) => Promise<void>; onNew: () => void }) { return <div className="h-full overflow-auto px-12 py-12"><div className="max-w-4xl"><div className="flex items-end justify-between border-b border-black/10 pb-5 dark:border-white/10"><div><p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#157252] dark:text-[#56d59d]">Inquiries</p><h1 className="mt-3 font-serif text-4xl tracking-[-.035em]">Questions worth returning to.</h1></div><button type="button" onClick={onNew} className="inline-flex items-center gap-2 text-xs font-semibold text-[#157252] dark:text-[#56d59d]"><MessageSquarePlus className="h-4 w-4" />New inquiry</button></div><div className="divide-y divide-black/10 border-b border-black/10 dark:divide-white/10 dark:border-white/10">{sessions.map((session) => <button type="button" key={session.id} onClick={() => void onOpen(session.id)} className="grid w-full grid-cols-[minmax(0,1fr)_auto] gap-5 py-5 text-left hover:bg-black/[.025] dark:hover:bg-white/[.035]"><div><b className="block truncate font-serif text-xl font-normal">{session.title}</b><span className="mt-2 block text-xs text-[#686a64] dark:text-[#aaa9a1]">Open the inquiry to continue asking against its existing context.</span></div><time className="self-center text-xs text-[#74766f] dark:text-[#85867f]">{formatDate(session.updatedAt)}</time></button>)}{!sessions.length && <div className="py-12 text-sm text-[#686a64] dark:text-[#aaa9a1]">No saved inquiries yet. Your related questions will appear here after the first answer is stored.</div>}</div></div></div>; }

type Automation = { name: string; title: string; description: string | null; enabled: boolean; triggerType: string; triggerDetail: string | null; path: string };
type AutomationRun = { id: string; automationName: string; status: string; trigger: string; attempt: number; startedAt: string | null; finishedAt: string | null; provider: string | null; model: string | null; output: string | null; errorCategory: string | null; errorMessage: string | null };
type AutomationDraft = { id: string; request: string; markdown: string; automation: Automation };
type AutomationArtifact = { id: string; runId: string; automationName: string; relativePath: string; sizeBytes: number; mediaType: string; liveView: boolean; outputKind: "artifact" | "live_view" | "notification"; contentJson: string | null; createdAt: string };

function AutomationsPage() {
  const [items, setItems] = useState<Automation[]>([]); const [runs, setRuns] = useState<AutomationRun[]>([]);
  const [artifacts, setArtifacts] = useState<AutomationArtifact[]>([]);
  const [request, setRequest] = useState(""); const [drafts, setDrafts] = useState<AutomationDraft[]>([]); const [busy, setBusy] = useState<string | null>(null); const [message, setMessage] = useState("");
  const [activeRunId, setActiveRunId] = useState<string | null>(null); const [liveEvents, setLiveEvents] = useState<Array<{kind:string;message:string}>>([]);
  const refresh = async () => { const [automations, history, outputs] = await Promise.all([invoke<Automation[]>("automation_list"), invoke<AutomationRun[]>("automation_list_runs", { name: null, before: null, limit: 30 }), invoke<AutomationArtifact[]>("automation_list_artifacts", { runId: null, limit: 50 })]); setItems(automations); setRuns(history); setArtifacts(outputs); };
  useEffect(() => { void refresh().catch(() => setMessage("Could not load automations.")); const disposers:Array<()=>void>=[]; listen("automation-run-updated", () => void refresh()).then((fn) => disposers.push(fn)).catch(() => {}); listen<{runId:string;event:{kind:string;message:string}}>("automation-run-event", ({payload}) => { setActiveRunId(payload.runId); setLiveEvents((current)=>[...current.slice(-199),payload.event]); }).then((fn)=>disposers.push(fn)).catch(()=>{}); return () => disposers.forEach((dispose)=>dispose()); }, []);
  const generate = async () => { if (!request.trim()) return; setBusy("draft"); setMessage("Dystil is searching your work and drafting automation options…"); try { const result = await invoke<AutomationDraft[]>("automation_draft", { request }); setDrafts(result); setMessage(result.length > 1 ? "Review these evidence-backed options and add the ones you want." : "Review the generated automation before adding it."); } catch (error) { setMessage(error instanceof Error ? error.message : String(error)); } finally { setBusy(null); } };
  const save = async (draft: AutomationDraft) => { setBusy(draft.id); try { await invoke("automation_save_draft", { draftId: draft.id }); setDrafts((current) => current.filter((item) => item.id !== draft.id)); setMessage("Automation added disabled. Enable it after reviewing its trigger."); await refresh(); } catch (error) { setMessage(error instanceof Error ? error.message : String(error)); } finally { setBusy(null); } };
  const run = async (name: string) => { setBusy(name); setActiveRunId(null); setLiveEvents([]); setMessage(`Running ${name}…`); try { const result = await invoke<AutomationRun>("automation_run_now", { name }); setMessage(result.status === "succeeded" ? result.output || "Automation completed." : result.errorMessage || "Automation failed."); await refresh(); } catch (error) { setMessage(error instanceof Error ? error.message : String(error)); } finally { setBusy(null); setActiveRunId(null); } };
  const cancel = async () => { if (!activeRunId) return; try { await invoke("automation_cancel", { runId: activeRunId }); setMessage("Automation cancelled."); } catch (error) { setMessage(error instanceof Error ? error.message : String(error)); } };
  const toggle = async (item: Automation) => { setBusy(item.name); try { await invoke("automation_set_enabled", { name: item.name, enabled: !item.enabled }); await refresh(); setMessage(`${item.title} ${item.enabled ? "disabled" : "enabled"}.`); } catch (error) { setMessage(error instanceof Error ? error.message : String(error)); } finally { setBusy(null); } };
  const remove = async (name: string) => { setBusy(name); try { await invoke("automation_delete", { name }); await refresh(); setMessage(`${name} removed.`); } catch (error) { setMessage(error instanceof Error ? error.message : String(error)); } finally { setBusy(null); } };
  return <div className="h-full overflow-auto px-12 py-12"><div className="max-w-5xl"><p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#157252] dark:text-[#56d59d]">Automations</p><h1 className="mt-3 font-serif text-4xl tracking-[-.035em]">Turn repeated work into a runner.</h1><p className="mt-3 max-w-2xl text-sm leading-6 text-[#686a64] dark:text-[#aaa9a1]">Describe what you want, or ask Dystil to suggest something from captured work. Nothing is added until you approve the generated Markdown.</p>
    <div className="mt-8 border-y border-black/10 py-5 dark:border-white/10"><textarea value={request} onChange={(event) => setRequest(event.target.value)} placeholder="Create a daily recap of tickets and messages, or suggest automations from my recent work…" className="min-h-24 w-full resize-y rounded-lg border border-black/15 bg-[#fffefa] p-4 text-sm outline-none focus:border-[#157252] dark:border-white/15 dark:bg-[#252725]" /><div className="mt-3 flex items-center justify-between gap-4"><p role="status" className="text-xs text-[#686a64] dark:text-[#aaa9a1]">{message}</p><Button onClick={() => void generate()} disabled={busy !== null || !request.trim()} className="rounded-lg">{busy === "draft" ? <Loader2 className="h-4 w-4 animate-spin" /> : <Zap className="h-4 w-4" />}Draft automation</Button></div></div>
    {drafts.map((draft) => <section key={draft.id} className="mt-6 border border-[#157252]/30 bg-[#eef3ed] p-5 dark:bg-[#1d2521]"><div className="flex items-start justify-between gap-5"><div><p className="text-[10px] font-bold uppercase tracking-[.1em] text-[#157252] dark:text-[#56d59d]">Approval required</p><h2 className="mt-2 font-serif text-2xl">{draft.automation.title}</h2><p className="mt-1 text-xs text-[#686a64] dark:text-[#aaa9a1]">{draft.automation.triggerType}{draft.automation.triggerDetail ? ` · ${draft.automation.triggerDetail}` : ""} · saved disabled</p></div><div className="flex gap-2"><Button variant="outline" onClick={() => setDrafts((current) => current.filter((item) => item.id !== draft.id))}>Discard</Button><Button onClick={() => void save(draft)} disabled={busy !== null}>{busy === draft.id && <Loader2 className="h-4 w-4 animate-spin" />}Add automation</Button></div></div><pre className="mt-4 max-h-72 overflow-auto whitespace-pre-wrap border-t border-black/10 pt-4 text-[11px] leading-5 dark:border-white/10">{draft.markdown}</pre></section>)}
    {busy && liveEvents.length > 0 && <section className="mt-6 border border-black/10 bg-[#252725] p-4 text-[#e8e8e2] dark:border-white/15"><div className="flex items-center justify-between"><p className="text-[10px] font-bold uppercase tracking-[.1em] text-[#56d59d]">Live run</p>{activeRunId&&<Button size="sm" variant="outline" onClick={()=>void cancel()}>Cancel</Button>}</div><div className="mt-3 max-h-52 overflow-auto font-mono text-[10px] leading-5">{liveEvents.map((event,index)=><p key={index}><span className="text-[#8fa99a]">{event.kind}</span> {event.message}</p>)}</div></section>}
    <section className="mt-10"><h2 className="font-serif text-2xl">Your automations</h2><div className="mt-4 divide-y divide-black/10 border-y border-black/10 dark:divide-white/10 dark:border-white/10">{items.map((item) => <div key={item.name} className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-5 py-5"><div><b className="font-serif text-xl font-normal">{item.title}</b><p className="mt-1 text-xs text-[#686a64] dark:text-[#aaa9a1]">{item.triggerType}{item.triggerDetail ? ` · ${item.triggerDetail}` : ""} · {item.enabled ? "enabled" : "disabled"}</p></div><div className="flex gap-2"><Button variant="ghost" onClick={() => void invoke("automation_open_definition", { name: item.name })}>Edit file</Button><Button variant="ghost" onClick={() => void toggle(item)} disabled={busy !== null}>{item.enabled ? "Disable" : "Enable"}</Button><Button variant="outline" onClick={() => void run(item.name)} disabled={busy !== null}>{busy === item.name ? <Loader2 className="h-4 w-4 animate-spin" /> : <Play className="h-4 w-4" />}Run now</Button><Button variant="ghost" aria-label={`Delete ${item.title}`} onClick={() => void remove(item.name)} disabled={busy !== null}><Trash2 className="h-4 w-4" /></Button></div></div>)}{!items.length && <p className="py-8 text-sm text-[#686a64] dark:text-[#aaa9a1]">No automations yet. Describe one above.</p>}</div></section>
    <section className="mt-10"><h2 className="font-serif text-2xl">Artifacts</h2><div className="mt-4 divide-y divide-black/10 border-y border-black/10 text-xs dark:divide-white/10 dark:border-white/10">{artifacts.map((artifact)=><ArtifactOutput key={artifact.id} artifact={artifact} runAutomation={run} disabled={busy!==null} />)}{!artifacts.length&&<p className="py-8 text-[#686a64] dark:text-[#aaa9a1]">Run outputs will appear here.</p>}</div></section>
    <section className="mt-10 pb-12"><h2 className="font-serif text-2xl">Recent runs</h2><div className="mt-4 divide-y divide-black/10 border-y border-black/10 text-xs dark:divide-white/10 dark:border-white/10">{runs.map((run) => <div key={run.id} className="grid grid-cols-[minmax(0,1fr)_100px_130px] gap-4 py-4"><span><b>{run.automationName}</b>{run.errorMessage && <small className="mt-1 block text-destructive">{run.errorCategory}: {run.errorMessage}</small>}</span><span>{run.status}{run.attempt > 1 ? ` · try ${run.attempt}` : ""}</span><span className="text-[#686a64] dark:text-[#aaa9a1]">{run.startedAt ? formatDate(run.startedAt) : "Queued"}</span></div>)}{!runs.length && <p className="py-8 text-[#686a64] dark:text-[#aaa9a1]">No runs yet.</p>}</div></section>
  </div></div>;
}

function ArtifactOutput({artifact,runAutomation,disabled}:{artifact:AutomationArtifact;runAutomation:(name:string)=>Promise<void>;disabled:boolean}) {
  let structured:Record<string,unknown>|null=null; try { structured=artifact.contentJson?JSON.parse(artifact.contentJson):null; } catch {}
  const actions=Array.isArray(structured?.actions)?structured.actions.filter((item):item is {label:string;automation:string}=>Boolean(item)&&typeof item==="object"&&typeof (item as {label?:unknown}).label==="string"&&typeof (item as {automation?:unknown}).automation==="string"):[];
  return <div className="py-4"><div className="flex items-start justify-between gap-4"><span><b>{typeof structured?.title==="string"?structured.title:artifact.relativePath}</b><small className="mt-1 block text-[#686a64] dark:text-[#aaa9a1]">{artifact.automationName} · {artifact.outputKind.replace("_"," ")}</small></span><Button size="sm" variant="ghost" onClick={()=>void invoke("automation_reveal_artifact",{artifactId:artifact.id})}>{artifact.outputKind==="artifact"?`${Math.ceil(artifact.sizeBytes/1024)} KB`:artifact.outputKind==="live_view"?"Live View":"Notification"}</Button></div>{typeof structured?.body==="string"&&<p className="mt-3 text-sm leading-6">{structured.body}</p>}{artifact.outputKind==="live_view"&&structured&&<pre className="mt-3 max-h-52 overflow-auto rounded bg-black/[.04] p-3 text-[10px] dark:bg-white/[.05]">{JSON.stringify(structured,null,2)}</pre>}{actions.length>0&&<div className="mt-3 flex flex-wrap gap-2">{actions.map((action)=><Button key={`${action.label}-${action.automation}`} size="sm" variant="outline" disabled={disabled} onClick={()=>void runAutomation(action.automation)}>{action.label}</Button>)}</div>}</div>;
}

function Composer({ input, setInput, busy, submit, mentionOpen, setMentionOpen, visiblePeers, choosePeer }: { input: string; setInput: (value: string) => void; busy: boolean; submit: () => Promise<void>; mentionOpen: boolean; setMentionOpen: (value: boolean) => void; visiblePeers: Peer[]; choosePeer: (peer: Peer) => void }) { return <div className="absolute bottom-7 left-[196px] right-0 z-10 mx-auto max-w-[900px] px-8"><div className="relative border border-black/15 bg-[#fffefa] p-2 shadow-[0_16px_40px_rgba(20,24,22,.16)] dark:border-white/15 dark:bg-[#252725] dark:shadow-[0_16px_40px_rgba(0,0,0,.28)]"><input value={input} onChange={(event) => { const value = event.target.value; setInput(value); if (value.includes("@")) setMentionOpen(true); }} onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); void submit(); } }} placeholder="Continue this inquiry…" className="h-11 w-full bg-transparent px-3 pr-12 text-sm outline-none placeholder:text-[#85867f]" /><button type="button" onClick={() => void submit()} disabled={busy || !input.trim()} className="absolute right-3 top-3 grid h-9 w-9 place-items-center rounded-md bg-[#157252] text-white disabled:opacity-35 dark:bg-[#56d59d] dark:text-[#151616]">{busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}</button>{mentionOpen && <div className="absolute bottom-[62px] left-0 w-full max-w-[390px] border border-black/15 bg-[#fffefa] p-2 shadow-xl dark:border-white/15 dark:bg-[#252725]"><p className="px-2 py-1 text-[10px] font-bold uppercase tracking-[.1em] text-[#74766f] dark:text-[#85867f]">Ask a teammate’s Dystil</p>{visiblePeers.length ? visiblePeers.map((peer) => <button type="button" key={peer.userId} onClick={() => choosePeer(peer)} className="flex w-full items-center gap-3 px-2 py-2 text-left hover:bg-black/[.04] dark:hover:bg-white/[.06]"><span className="grid h-7 w-7 place-items-center rounded-full bg-[#dfe8df] text-[10px] font-semibold text-[#157252] dark:bg-[#173a30] dark:text-[#56d59d]">{(peer.displayName || peer.email).slice(0, 2).toUpperCase()}</span><span className="min-w-0 flex-1"><b className="block text-xs">{peer.displayName || peer.email.split("@")[0]}</b><span className="block truncate text-[11px] text-[#686a64] dark:text-[#aaa9a1]">{peer.email}</span></span></button>) : <p className="p-2 text-xs text-[#686a64] dark:text-[#aaa9a1]">No compatible teammates are available yet.</p>}</div>}</div><p className="flex justify-between px-1 pt-2 text-[11px] text-[#686a64] dark:text-[#aaa9a1]"><span><AtSign className="mr-1 inline h-3 w-3" />Ask a teammate</span><span>Enter to continue inquiry</span></p></div>; }

function LocalAnswer({ chat }: { chat: Chat }) {
  if (chat.status === "pending") return <div className="border-l border-[#157252] py-2 pl-4 text-sm text-[#686a64] dark:border-[#56d59d] dark:text-[#aaa9a1]"><Loader2 className="mr-2 inline h-4 w-4 animate-spin" />Searching the relevant local work…</div>;
  const provider = chat.provider === "claude" ? "Claude Code" : chat.provider === "codex" ? "Codex" : chat.provider;
  const answer = chat.status === "failed" && chat.historical
    ? "This earlier attempt did not complete. The rest of this inquiry and its saved sources are still available."
    : chat.answer || "Dystil could not generate an answer from this local context.";
  return <div className="border-l border-black/15 py-1 pl-4 dark:border-white/15"><p className={cn("text-sm leading-6", chat.status === "failed" && "text-destructive")}>{answer}</p>{chat.citations?.length ? <div className="mt-4 border-t border-black/10 pt-3 text-xs text-[#686a64] dark:border-white/10 dark:text-[#aaa9a1]"><b className="mb-1 block text-[10px] font-bold uppercase tracking-[.1em]">Available sources</b>{chat.citations.map((item) => <p key={`${item.label}-${item.localDate}`}>• {item.label}{item.localDate ? ` — ${item.localDate}` : ""}</p>)}</div> : null}{provider ? <p className="mt-3 text-[10px] text-[#74766f] dark:text-[#85867f]">Answered by {provider}{chat.model ? ` · ${chat.model}` : ""}</p> : null}</div>;
}

function TeamAnswer({ messages, peers }: { messages: AgentMessage[]; peers: Peer[] }) { const answer = messages.find((message) => message.kind === "response"); if (!answer) return <div className="border-l border-[#157252] py-2 pl-4 text-sm text-[#686a64] dark:border-[#56d59d] dark:text-[#aaa9a1]"><Loader2 className="mr-2 inline h-4 w-4 animate-spin" />Waiting for a teammate’s Dystil…</div>; const name = peers.find((peer) => peer.userId === answer.peerUserId)?.displayName || "Teammate"; return <div className="border-l border-black/15 py-1 pl-4 dark:border-white/15"><p className="text-sm leading-6">{answer.text}</p><p className="mt-3 text-[10px] font-bold uppercase tracking-[.1em] text-[#74766f] dark:text-[#85867f]">Evidence from {name}’s local work</p>{answer.evidence.map((item) => <p key={item.label} className="mt-1 text-xs text-[#686a64] dark:text-[#aaa9a1]">• {item.label} — {item.localDate}</p>)}</div>; }

function SettingsPage({ recording, toggling, onToggle, screenshots, onScreenshot, screenshotBusy, userName, userEmail, onLogout, loggingOut, version }: { recording: boolean; toggling: boolean; onToggle: () => void; screenshots: boolean; onScreenshot: (value: boolean) => void; screenshotBusy: boolean; userName: string; userEmail: string; onLogout: () => void; loggingOut: boolean; version: string }) { return <div className="h-full overflow-auto px-12 py-12"><div className="max-w-3xl"><p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#157252] dark:text-[#56d59d]">Local memory</p><h1 className="mt-3 font-serif text-4xl tracking-[-.035em]">Capture settings</h1><p className="mt-3 text-sm text-[#686a64] dark:text-[#aaa9a1]">Control what Dystil can retain on this device.</p><div className="mt-10 divide-y divide-black/10 border-y border-black/10 dark:divide-white/10 dark:border-white/10"><SettingRow label="Local capture" description={recording ? "Dystil is capturing locally." : "Capture is paused."} action={<Button onClick={onToggle} disabled={toggling} variant="outline" className="rounded-lg">{toggling ? <Loader2 className="h-4 w-4 animate-spin" /> : recording ? <Pause className="h-4 w-4" /> : <Play className="h-4 w-4" />}{recording ? "Pause" : "Resume"}</Button>} /><SettingRow label="Capture screenshots" description="Off by default. Accessibility-only capture remains available." action={screenshotBusy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Switch aria-label="Capture screenshots" checked={screenshots} onCheckedChange={onScreenshot} />} /><SettingRow label="Account" description={`${userName} · ${userEmail}`} action={<Button variant="outline" onClick={onLogout} disabled={loggingOut} className="rounded-lg">{loggingOut ? <Loader2 className="h-4 w-4 animate-spin" /> : "Sign out"}</Button>} /></div><AiPresetSettings /><ExternalMcpSettings />{version && <p className="mt-5 text-xs text-[#74766f] dark:text-[#85867f]">Dystil v{version}</p>}</div></div>; }

type AiPreset = { id: string; name: string; providerKind: "codex" | "claude" | "openai_compatible" | "ollama"; endpoint: string | null; model: string; active: boolean; credentialPresent: boolean; validationStatus: "unknown" | "ready" | "error"; validationMessage: string | null };
type LocalPresetProvider = "ollama" | "openai_compatible";

function AiPresetSettings() {
  const [presets, setPresets] = useState<AiPreset[]>([]);
  const [statuses, setStatuses] = useState<Record<ManagedProvider, ManagedProviderStatus | null>>({ codex: null, claude: null });
  const [provider, setProvider] = useState<LocalPresetProvider>("ollama");
  const [name, setName] = useState("Local Ollama");
  const [endpoint, setEndpoint] = useState("http://localhost:11434/v1");
  const [model, setModel] = useState("");
  const [models, setModels] = useState<string[]>([]);
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const refresh = async () => {
    try {
      const [items, codex, claude] = await Promise.all([
        invoke<AiPreset[]>("ai_preset_list"),
        invoke<ManagedProviderStatus>("ai_provider_status", { provider: "codex" }),
        invoke<ManagedProviderStatus>("ai_provider_status", { provider: "claude" }),
      ]);
      setPresets(items); setStatuses({ codex, claude });
    } catch { setMessage("Could not read AI presets."); }
  };
  useEffect(() => { void refresh(); }, []);
  const chooseProvider = (next: LocalPresetProvider) => {
    setProvider(next); setModels([]); setModel(""); setApiKey("");
    setName(next === "ollama" ? "Local Ollama" : "Personal AI");
    setEndpoint(next === "ollama" ? "http://localhost:11434/v1" : "https://api.openai.com/v1");
  };
  const connectManaged = async (kind: ManagedProvider) => {
    setBusy(kind); setMessage("");
    try {
      const status = statuses[kind];
      if (status?.state !== "ready") await invoke("ai_provider_install", { provider: kind });
      if (!status?.authenticated) await invoke("ai_provider_login", { provider: kind });
      const next = await invoke<ManagedProviderStatus>("ai_provider_test", { provider: kind });
      if (!next.authenticated) throw new Error("Finish provider sign-in, then check again.");
      const available = await invoke<ManagedProviderModel[]>("ai_provider_models", { provider: kind }).catch(() => []);
      await invoke("ai_preset_activate_managed", { providerKind: kind, model: available.find((item) => item.isDefault)?.id || available[0]?.id || "default" });
      setMessage(`${kind === "codex" ? "ChatGPT subscription" : "Claude subscription"} is now active.`); await refresh();
    } catch (error) { setMessage(error instanceof Error ? error.message : String(error || "Could not connect this subscription.")); }
    finally { setBusy(null); }
  };
  const disconnectManaged = async (kind: ManagedProvider) => {
    setBusy(`${kind}-logout`); setMessage("");
    try {
      await invoke<ManagedProviderStatus>("ai_provider_logout", { provider: kind });
      setMessage(`${kind === "codex" ? "Codex" : "Claude Code"} signed out. Choose Connect to use another account.`);
      await refresh();
    } catch (error) { setMessage(error instanceof Error ? error.message : String(error || "Could not sign out.")); }
    finally { setBusy(null); }
  };
  const discover = async () => {
    setBusy("discover"); setMessage("");
    try {
      const result = await invoke<{ models: string[]; detail: string }>("ai_preset_discover_models", { providerKind: provider, endpoint, apiKey: apiKey || null });
      setModels(result.models); if (!model && result.models[0]) setModel(result.models[0]); setMessage(result.detail);
    } catch (error) { setMessage(error instanceof Error ? error.message : String(error || "Could not discover models.")); }
    finally { setBusy(null); }
  };
  const save = async (event: React.FormEvent) => {
    event.preventDefault(); setBusy("save"); setMessage("");
    try {
      const saved = await invoke<AiPreset>("ai_preset_save", { name, providerKind: provider, endpoint, model, apiKey: apiKey || null });
      setApiKey(""); setMessage("Preset saved. Preparing its constrained AI runtime…"); await refresh();
      await invoke("ai_preset_test", { presetId: saved.id }); setMessage("Preset is connected and active."); await refresh();
    } catch (error) { setMessage(error instanceof Error ? error.message : String(error || "Could not save this preset.")); await refresh(); }
    finally { setBusy(null); }
  };
  const activate = async (item: AiPreset) => {
    setBusy(item.id); setMessage("");
    try {
      if (item.providerKind === "codex" || item.providerKind === "claude") await invoke("ai_preset_activate_managed", { providerKind: item.providerKind, model: item.model });
      else { await invoke("ai_preset_activate", { presetId: item.id }); await invoke("ai_preset_test", { presetId: item.id }); }
      setMessage(`${item.name} is now active.`); await refresh();
    } catch (error) { setMessage(error instanceof Error ? error.message : String(error || "Could not activate this preset.")); }
    finally { setBusy(null); }
  };
  const remove = async (item: AiPreset) => {
    setBusy(item.id); setMessage("");
    try { await invoke("ai_preset_delete", { presetId: item.id }); setMessage(`${item.name} was removed.`); await refresh(); }
    catch (error) { setMessage(error instanceof Error ? error.message : String(error || "Could not remove this preset.")); }
    finally { setBusy(null); }
  };
  const localPresets = presets.filter((item) => item.providerKind === "ollama" || item.providerKind === "openai_compatible");
  return <section className="mt-10 border-t border-black/10 pt-8 dark:border-white/10">
    <p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#157252] dark:text-[#56d59d]">Intelligence</p>
    <h2 className="mt-2 font-serif text-2xl font-normal tracking-[-.02em]">AI presets</h2>
    <p className="mt-2 max-w-[64ch] text-xs leading-5 text-[#686a64] dark:text-[#aaa9a1]">Choose one connection for Dystil inquiries. Subscription presets use the provider’s official client. Ollama and custom endpoints run through a private, tool-disabled Pi runtime; your work remains bounded by Dystil before inference.</p>
    <div className="mt-5 divide-y divide-black/10 border-y border-black/10 dark:divide-white/10 dark:border-white/10">
      {(["codex", "claude"] as const).map((kind) => {
        const item = presets.find((preset) => preset.providerKind === kind); const status = statuses[kind]; const ready = status?.state === "ready" && status.authenticated;
        const label = kind === "codex" ? "ChatGPT subscription" : "Claude subscription";
        return <div key={kind} className="flex items-center gap-5 py-4"><div className="min-w-0 flex-1"><div className="flex items-center gap-2"><b className="text-sm">{label}</b>{item?.active && <span className="text-[10px] font-semibold text-[#157252] dark:text-[#56d59d]">Active</span>}</div><p className="mt-1 text-xs text-[#686a64] dark:text-[#aaa9a1]">{ready ? `Connected through the official ${kind === "codex" ? "Codex" : "Claude Code"} client.` : "Install and sign in with your existing subscription."}</p></div><div className="flex gap-2">{ready && <Button type="button" variant="outline" className="rounded-lg" disabled={busy !== null} onClick={() => void disconnectManaged(kind)}>{busy === `${kind}-logout` && <Loader2 className="h-4 w-4 animate-spin" />}Sign out</Button>}<Button type="button" variant={item?.active && ready ? "outline" : "default"} className="rounded-lg" disabled={busy !== null || Boolean(item?.active && ready)} onClick={() => void connectManaged(kind)}>{busy === kind && <Loader2 className="h-4 w-4 animate-spin" />}{item?.active && ready ? "In use" : ready ? "Use" : "Connect"}</Button></div></div>;
      })}
      {localPresets.map((item) => <div key={item.id} className="flex items-center gap-5 py-4"><div className="min-w-0 flex-1"><div className="flex items-center gap-2"><b className="truncate text-sm">{item.name}</b>{item.active && <span className="text-[10px] font-semibold text-[#157252] dark:text-[#56d59d]">Active</span>}</div><p className="mt-1 truncate text-xs text-[#686a64] dark:text-[#aaa9a1]">{item.providerKind === "ollama" ? "Ollama" : item.endpoint} · {item.model}{item.validationStatus === "error" ? " · Needs attention" : item.validationStatus === "ready" ? " · Checked" : ""}</p></div><div className="flex gap-2"><Button type="button" variant="outline" className="rounded-lg" disabled={busy !== null} onClick={() => void remove(item)}>Remove</Button><Button type="button" className="rounded-lg" disabled={busy !== null || item.active} onClick={() => void activate(item)}>{busy === item.id && <Loader2 className="h-4 w-4 animate-spin" />}{item.active ? "In use" : "Use"}</Button></div></div>)}
    </div>
    <form onSubmit={save} className="mt-6 border-y border-black/10 py-5 dark:border-white/10">
      <div className="flex items-end justify-between gap-5"><div><b className="text-sm">Add a local or custom preset</b><p className="mt-1 text-xs text-[#686a64] dark:text-[#aaa9a1]">Ollama needs no key. Custom keys are stored only in your operating system credential store.</p></div><Select value={provider} onValueChange={(value) => chooseProvider(value as LocalPresetProvider)}><SelectTrigger aria-label="Preset provider" className="h-9 w-48 rounded-lg"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="ollama">Ollama</SelectItem><SelectItem value="openai_compatible">OpenAI-compatible</SelectItem></SelectContent></Select></div>
      <div className="mt-4 grid gap-3 sm:grid-cols-2"><label className="grid gap-1 text-xs font-semibold">Preset name<input value={name} onChange={(event) => setName(event.target.value)} required className="h-9 rounded-md border border-black/15 bg-transparent px-2 font-normal outline-none focus:border-[#157252] dark:border-white/15" /></label><label className="grid gap-1 text-xs font-semibold">Endpoint<input value={endpoint} onChange={(event) => setEndpoint(event.target.value)} required className="h-9 rounded-md border border-black/15 bg-transparent px-2 font-normal outline-none focus:border-[#157252] dark:border-white/15" /></label></div>
      <div className="mt-3 grid gap-3 sm:grid-cols-[minmax(0,1fr)_auto]"><label className="grid gap-1 text-xs font-semibold">Model{models.length ? <Select value={model} onValueChange={setModel}><SelectTrigger className="h-9 rounded-md"><SelectValue placeholder="Choose a model" /></SelectTrigger><SelectContent>{models.map((item) => <SelectItem key={item} value={item}>{item}</SelectItem>)}</SelectContent></Select> : <input value={model} onChange={(event) => setModel(event.target.value)} placeholder={provider === "ollama" ? "e.g. qwen3:8b" : "Model ID"} required className="h-9 rounded-md border border-black/15 bg-transparent px-2 font-normal outline-none focus:border-[#157252] dark:border-white/15" />}</label><Button type="button" variant="outline" className="self-end rounded-lg" disabled={busy !== null || (provider === "openai_compatible" && !apiKey)} onClick={() => void discover()}>{busy === "discover" && <Loader2 className="h-4 w-4 animate-spin" />}Find models</Button></div>
      {provider === "openai_compatible" && <label className="mt-3 grid gap-1 text-xs font-semibold">API key<input type="password" autoComplete="off" value={apiKey} onChange={(event) => setApiKey(event.target.value)} required className="h-9 rounded-md border border-black/15 bg-transparent px-2 font-normal outline-none focus:border-[#157252] dark:border-white/15" /></label>}
      <Button type="submit" className="mt-4 rounded-lg" disabled={busy !== null || !model.trim() || (provider === "openai_compatible" && !apiKey.trim())}>{busy === "save" && <Loader2 className="h-4 w-4 animate-spin" />}Save, check & use</Button>
    </form>
    {message && <p role="status" className="mt-3 text-xs leading-5 text-[#686a64] dark:text-[#aaa9a1]">{message}</p>}
  </section>;
}

type ManagedProvider = "codex" | "claude";
type ManagedProviderStatus = { provider: ManagedProvider; state: string; installedVersion: string | null; authenticated: boolean | null; detail: string | null };
type ManagedProviderModel = { id: string; displayName: string; description: string; isDefault: boolean };

function ManagedProviderSettings() {
  const [statuses, setStatuses] = useState<Record<ManagedProvider, ManagedProviderStatus | null>>({ codex: null, claude: null });
  const [models, setModels] = useState<Record<ManagedProvider, ManagedProviderModel[]>>({ codex: [], claude: [] });
  const [providerModels, setProviderModels] = useState<Record<ManagedProvider, string>>({ codex: "default", claude: "default" });
  const [modelsLoading, setModelsLoading] = useState(false);
  const [selected, setSelected] = useState<ManagedProvider>("codex");
  const [personalActive, setPersonalActive] = useState(false);
  const [busy, setBusy] = useState<ManagedProvider | null>(null);
  const [message, setMessage] = useState("");
  const [claudeCodeRequired, setClaudeCodeRequired] = useState(false);
  const [claudeAuthorizationCode, setClaudeAuthorizationCode] = useState("");
  const refresh = async () => {
    try {
      const [codex, claude, presets] = await Promise.all([
        invoke<ManagedProviderStatus>("ai_provider_status", { provider: "codex" }),
        invoke<ManagedProviderStatus>("ai_provider_status", { provider: "claude" }),
        invoke<AiPreset[]>("ai_preset_list"),
      ]);
      const activeManaged = presets.find((item) => item.active && (item.providerKind === "codex" || item.providerKind === "claude"));
      if (activeManaged) setSelected(activeManaged.providerKind as ManagedProvider);
      setStatuses({ codex, claude }); setPersonalActive(presets.some((item) => item.active && (item.providerKind === "ollama" || item.providerKind === "openai_compatible")));
      setModelsLoading(true);
      const [codexModels, claudeModels] = await Promise.all([
        codex.state === "ready" ? invoke<ManagedProviderModel[]>("ai_provider_models", { provider: "codex" }).catch(() => []) : [],
        claude.state === "ready" ? invoke<ManagedProviderModel[]>("ai_provider_models", { provider: "claude" }).catch(() => []) : [],
      ]);
      setModels({ codex: codexModels, claude: claudeModels });
      setProviderModels((current) => ({
        codex: activeManaged?.providerKind === "codex"
          ? activeManaged.model
          : modelAvailable(current.codex, codexModels),
        claude: activeManaged?.providerKind === "claude"
          ? activeManaged.model
          : modelAvailable(current.claude, claudeModels),
      }));
      setModelsLoading(false);
    } catch { setMessage("Could not read AI connection status."); }
  };
  useEffect(() => {
    void refresh();
    window.addEventListener("dystil-ai-presets-changed", refresh);
    let unlisten: (() => void) | undefined;
    listen("ai-provider-login-updated", () => void refresh()).then((dispose) => { unlisten = dispose; }).catch(() => undefined);
    return () => { window.removeEventListener("dystil-ai-presets-changed", refresh); unlisten?.(); };
  }, []);
  const connect = async (provider: ManagedProvider) => {
    setBusy(provider); setMessage("");
    try {
      const status = statuses[provider];
      if (status?.state !== "ready") await invoke("ai_provider_install", { provider });
      const loginMode = await invoke<string>("ai_provider_login", { provider });
      setClaudeCodeRequired(provider === "claude" && loginMode === "codeRequired");
      setMessage(provider === "claude" && loginMode === "codeRequired"
        ? "Finish signing in in the browser, then paste the authorization code below."
        : `Continue ${provider === "codex" ? "Codex" : "Claude Code"} sign-in in the browser, then choose Check connection.`);
    } catch (error) { setMessage(error instanceof Error ? error.message : String(error || "Could not start provider sign-in.")); }
    finally { setBusy(null); await refresh(); }
  };
  const completeClaudeLogin = async () => {
    if (!claudeAuthorizationCode.trim()) return;
    setBusy("claude"); setMessage("");
    try {
      const status = await invoke<ManagedProviderStatus>("ai_provider_complete_claude_login", { authorizationCode: claudeAuthorizationCode });
      setStatuses((current) => ({ ...current, claude: status }));
      setClaudeAuthorizationCode(""); setClaudeCodeRequired(false);
      await refresh();
      setMessage("Claude Code is connected. Choose Use for chat to make it Dystil’s managed chat provider.");
    } catch (error) { setMessage(error instanceof Error ? error.message : String(error || "Claude Code sign-in could not be completed.")); }
    finally { setBusy(null); }
  };
  const test = async (provider: ManagedProvider) => {
    setBusy(provider); setMessage("");
    try { await invoke<ManagedProviderStatus>("ai_provider_test", { provider }); await refresh(); setMessage(`${provider === "codex" ? "Codex" : "Claude Code"} is ready for Dystil inquiries.`); }
    catch (error) { setMessage(error instanceof Error ? error.message : "Connection check failed."); }
    finally { setBusy(null); }
  };
  const select = async (provider: ManagedProvider) => {
    setBusy(provider); setMessage("");
    const nextModel = modelAvailable(providerModels[provider], models[provider]);
    try { await invoke("ai_preset_activate_managed", { providerKind: provider, model: nextModel }); setSelected(provider); setProviderModels((current) => ({ ...current, [provider]: nextModel })); setMessage(`${provider === "codex" ? "Codex" : "Claude Code"} will answer inquiries.`); }
    catch (error) { setMessage(error instanceof Error ? error.message : "Could not select this provider."); }
    finally { setBusy(null); }
  };
  const changeModel = async (provider: ManagedProvider, nextModel: string) => {
    setProviderModels((current) => ({ ...current, [provider]: nextModel }));
    if (provider !== selected) {
      setMessage(`Model chosen for ${provider === "codex" ? "Codex" : "Claude Code"}. Choose Use for chat to activate it.`);
      return;
    }
    setBusy(provider); setMessage("");
    try {
      await invoke("ai_preset_activate_managed", { providerKind: provider, model: nextModel });
      setMessage(`${models[provider].find((item) => item.id === nextModel)?.displayName || nextModel} will be used for managed chat.`);
    } catch (error) { setMessage(error instanceof Error ? error.message : String(error || "Could not save the managed model.")); }
    finally { setBusy(null); }
  };
  return <section className="mt-10 border-t border-black/10 pt-8 dark:border-white/10">
    <p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#157252] dark:text-[#56d59d]">Managed AI</p>
    <h2 className="mt-2 font-serif text-2xl font-normal tracking-[-.02em]">Connect Codex or Claude Code</h2>
    <p className="mt-2 max-w-[64ch] text-xs leading-5 text-[#686a64] dark:text-[#aaa9a1]">Dystil installs the official CLI privately, opens its own browser sign-in, and gives that process only Dystil’s bounded local retrieval tools for an inquiry. Dystil never receives the provider account token.</p>
    {personalActive && <p className="mt-4 border-y border-black/10 py-3 text-xs leading-5 text-[#686a64] dark:border-white/10 dark:text-[#aaa9a1]">A local or custom preset is currently active.</p>}
    <div className="mt-5 divide-y divide-black/10 border-y border-black/10 dark:divide-white/10 dark:border-white/10">{(["codex", "claude"] as const).map((provider) => {
      const status = statuses[provider];
      const name = provider === "codex" ? "Codex" : "Claude Code";
      const ready = status?.state === "ready" && status.authenticated;
      const repair = status?.state === "repairRequired";
      const active = selected === provider && !personalActive;
      return <div key={provider} className="py-5">
        <div className="grid grid-cols-[minmax(0,1fr)_auto] gap-4">
          <div>
            <div className="flex items-center gap-2">
              <b className="text-sm">{name}</b>
              {ready && <span className="inline-flex items-center gap-1.5 text-[10px] font-semibold text-[#157252] dark:text-[#56d59d]"><i className="h-1.5 w-1.5 rounded-full bg-current" />Connected</span>}
              {active && <span className="text-[10px] text-[#686a64] dark:text-[#aaa9a1]">· Used for chat</span>}
            </div>
            <p className="mt-1 text-xs text-[#686a64] dark:text-[#aaa9a1]">{ready ? "Signed in and ready for local inquiries." : status?.state === "ready" ? "Installed; sign-in is still required." : repair ? "Installation is incomplete. Dystil can repair it without changing your other provider settings." : "Not installed yet."}</p>
          </div>
          <div className="flex items-center gap-2">{ready ? <><Button type="button" variant="outline" className="rounded-lg" disabled={busy !== null} onClick={() => void test(provider)}>{busy === provider ? <Loader2 className="h-4 w-4 animate-spin" /> : "Check"}</Button><Button type="button" className="rounded-lg" disabled={busy !== null || active} onClick={() => void select(provider)}>{active ? "In use" : "Use for chat"}</Button></> : <Button type="button" className="rounded-lg" disabled={busy !== null} onClick={() => void connect(provider)}>{busy === provider ? <Loader2 className="h-4 w-4 animate-spin" /> : status?.state === "ready" ? "Sign in" : repair ? "Repair & sign in" : "Install & sign in"}</Button>}</div>
        </div>
        {ready && <ManagedModelPicker
          provider={provider}
          models={models[provider]}
          value={providerModels[provider]}
          loading={modelsLoading}
          disabled={busy !== null}
          active={active}
          onChange={(nextModel) => void changeModel(provider, nextModel)}
        />}
      </div>;
    })}</div>
    {claudeCodeRequired && <form className="mt-4 flex max-w-xl items-end gap-2" onSubmit={(event) => { event.preventDefault(); void completeClaudeLogin(); }}><label className="grid min-w-0 flex-1 gap-1 text-xs font-semibold">Claude authorization code<input autoFocus value={claudeAuthorizationCode} onChange={(event) => setClaudeAuthorizationCode(event.target.value)} autoComplete="off" placeholder="Paste the code shown after browser sign-in" className="h-9 rounded-md border border-black/15 bg-transparent px-2 font-normal outline-none focus:border-[#157252] dark:border-white/15 dark:focus:border-[#56d59d]" /></label><Button type="submit" className="rounded-lg" disabled={busy !== null || !claudeAuthorizationCode.trim()}>{busy === "claude" ? <Loader2 className="h-4 w-4 animate-spin" /> : "Complete sign-in"}</Button></form>}
    {message && <p role="status" className="mt-3 text-xs leading-5 text-[#686a64] dark:text-[#aaa9a1]">{message}</p>}
  </section>;
}

function modelAvailable(current: string, models: ManagedProviderModel[]) {
  return models.some((item) => item.id === current)
    ? current
    : models.find((item) => item.isDefault)?.id || models[0]?.id || "default";
}

function ManagedModelPicker({ provider, models, value, loading, disabled, active, onChange }: {
  provider: ManagedProvider;
  models: ManagedProviderModel[];
  value: string;
  loading: boolean;
  disabled: boolean;
  active: boolean;
  onChange: (value: string) => void;
}) {
  const selectedModel = models.find((item) => item.id === value);
  const providerName = provider === "codex" ? "Codex" : "Claude Code";
  return <div className="mt-4 grid gap-3 border-t border-black/10 pt-4 dark:border-white/10 sm:grid-cols-[minmax(0,1fr)_minmax(230px,300px)] sm:items-center">
    <div>
      <label className="text-xs font-semibold" htmlFor={`${provider}-model`}>Chat model</label>
      <p className="mt-1 max-w-[46ch] text-[11px] leading-5 text-[#74766f] dark:text-[#85867f]">
        {loading ? `Reading models from ${providerName}…` : selectedModel?.description || "Choose which model this connection should use."}
      </p>
    </div>
    <Select value={value} onValueChange={onChange} disabled={loading || disabled || !models.length}>
      <SelectTrigger id={`${provider}-model`} aria-label={`${providerName} chat model`} className="h-11 rounded-lg border-black/15 bg-[#fffefa] px-3 font-sans text-[13px] shadow-none focus:border-[#157252] dark:border-white/15 dark:bg-[#252725] dark:focus:border-[#56d59d]">
        <SelectValue placeholder={loading ? "Loading models…" : "Choose a model"} />
      </SelectTrigger>
      <SelectContent position="popper" sideOffset={6} className="max-h-[320px] rounded-lg border-black/15 bg-[#fffefa] p-1 text-[#242522] shadow-[0_14px_34px_rgba(20,24,22,.18)] dark:border-white/15 dark:bg-[#252725] dark:text-[#f1f0eb]">
        {models.map((item) => <SelectItem key={item.id} value={item.id} className="min-h-10 rounded-md py-2 pl-8 pr-3 font-sans text-[13px] focus:bg-[#dfe8df] focus:text-[#173c2b] dark:focus:bg-white/[.08] dark:focus:text-[#f1f0eb]">{item.displayName}</SelectItem>)}
      </SelectContent>
    </Select>
    {!active && <p className="text-[10px] text-[#74766f] dark:text-[#85867f] sm:col-start-2">Choose Use for chat to activate this model.</p>}
  </div>;
}

function ExternalMcpSettings() {
  const [consented, setConsented] = useState(false);
  const [busy, setBusy] = useState<"codex" | "claude" | null>(null);
  const [message, setMessage] = useState("");
  const add = async (client: "codex" | "claude") => {
    if (!consented || busy) return;
    setBusy(client); setMessage("");
    try {
      const result = await invoke<{ detail: string }>("external_mcp_add", { client });
      setMessage(`${result.detail} In Codex or Claude Code, start a new session before asking about your work.`);
    } catch (error) { setMessage(error instanceof Error ? error.message : `Could not add Dystil to ${client}.`); }
    finally { setBusy(null); }
  };
  return <section className="mt-10 border-t border-black/10 pt-8 dark:border-white/10">
    <p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#157252] dark:text-[#56d59d]">External AI</p>
    <h2 className="mt-2 font-serif text-2xl font-normal tracking-[-.02em]">Use Dystil from Codex or Claude Code</h2>
    <p className="mt-2 max-w-[64ch] text-xs leading-5 text-[#686a64] dark:text-[#aaa9a1]">Add Dystil’s read-only local search tools to your own terminal or IDE assistant. This is separate from Dystil’s in-app chat connections.</p>
    <label className="mt-5 flex max-w-[64ch] cursor-pointer items-start gap-3 border-y border-black/10 py-4 text-xs leading-5 text-[#686a64] dark:border-white/10 dark:text-[#aaa9a1]">
      <Checkbox checked={consented} onCheckedChange={(checked) => setConsented(checked === true)} aria-label="Allow external AI access to Dystil's sanitized local data" />
      <span>I understand that the selected external AI client will be able to read Dystil’s sanitized activity evidence. Screenshots, raw accessibility trees, writes, shell access, and arbitrary database access are not shared. Codex setup also adds a marked Dystil preference to its existing global guidance; it never replaces your instructions.</span>
    </label>
    <div className="mt-4 flex flex-wrap gap-3"><Button type="button" className="rounded-lg" disabled={!consented || busy !== null} onClick={() => void add("codex")}>{busy === "codex" && <Loader2 className="h-4 w-4 animate-spin" />}Add Dystil to Codex</Button><Button type="button" variant="outline" className="rounded-lg" disabled={!consented || busy !== null} onClick={() => void add("claude")}>{busy === "claude" && <Loader2 className="h-4 w-4 animate-spin" />}Add Dystil to Claude Code</Button></div>
    <p className="mt-3 text-[11px] leading-5 text-[#74766f] dark:text-[#85867f]">Requires the selected client’s own CLI to be installed on this computer. Dystil updates only that client’s MCP configuration.</p>
    {message && <p role="status" className="mt-3 text-xs leading-5 text-[#686a64] dark:text-[#aaa9a1]">{message}</p>}
  </section>;
}

function SettingRow({ label, description, action }: { label: string; description: string; action: React.ReactNode }) { return <div className="flex items-center gap-6 py-5"><div className="min-w-0 flex-1"><b className="text-sm">{label}</b><p className="mt-1 text-xs text-[#686a64] dark:text-[#aaa9a1]">{description}</p></div>{action}</div>; }
function formatDate(value: string) { const date = new Date(`${value}Z`); return Number.isNaN(date.getTime()) ? "Recently" : date.toLocaleDateString([], { month: "short", day: "numeric" }); }

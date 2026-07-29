"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  AtSign, ChevronRight, Clock3, FileText, FolderSearch, Loader2, MessageSquarePlus,
  Pause, Play, Search, Send, Settings2,
} from "lucide-react";
import { DystilBrand } from "@/components/dystil-brand";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";

type Card = { windowId: string; title: string; summary: string; applications: string[]; startTime: string; endTime: string; status: string; lastObservedState: string };
type Peer = { userId: string; displayName: string | null; email: string; agentStatus: string };
type AgentMessage = { messageId: string; peerUserId: string; direction: string; kind: string; localStatus: string; text: string; evidence: Array<{ label: string; localDate: string }> };
type View = "ask" | "inquiry" | "index" | "inquiries" | "settings";

export type ChatSession = { id: string; title: string; updatedAt: string };
export type Chat = { id: string; conversationId: string; question: string; mode: "local" | "team"; answer?: string | null; status?: "pending" | "complete" | "failed"; citations?: Array<{ label: string; localDate: string }>; provider?: string | null; model?: string | null; elapsedMs?: number | null; historical?: boolean };

type Props = {
  userName: string; userEmail: string; recording: boolean; toggling: boolean; onToggleRecording: () => void;
  screenshotEnabled: boolean; onScreenshotChange: (value: boolean) => void; screenshotBusy: boolean;
  peers: Peer[]; agentMessages: AgentMessage[]; cards: Card[]; loadingCards: boolean; sessions: ChatSession[];
  onLoadSession: (sessionId: string) => Promise<Chat[]>; onSendLocal: (sessionId: string, question: string) => Promise<Chat>;
  onAskPeer: (peerId: string, question: string) => Promise<void>; onLogout: () => void; loggingOut: boolean; version: string;
};

export function ChatShell({ userName, userEmail, recording, toggling, onToggleRecording, screenshotEnabled, onScreenshotChange, screenshotBusy, peers, agentMessages, cards, loadingCards, sessions, onLoadSession, onSendLocal, onAskPeer, onLogout, loggingOut, version }: Props) {
  const [view, setView] = useState<View>("ask");
  const [input, setInput] = useState("");
  const [mentionOpen, setMentionOpen] = useState(false);
  const [selectedPeer, setSelectedPeer] = useState<Peer | null>(null);
  const [busy, setBusy] = useState(false);
  const [loadingSession, setLoadingSession] = useState(false);
  const [activeInquiryId, setActiveInquiryId] = useState(() => crypto.randomUUID());
  const [turns, setTurns] = useState<Chat[]>([]);
  const [selectedCardId, setSelectedCardId] = useState<string | null>(null);
  const trailEnd = useRef<HTMLDivElement>(null);
  const activeInquiry = sessions.find((session) => session.id === activeInquiryId);
  const selectedCard = cards.find((card) => card.windowId === selectedCardId) || cards[0];
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
  useEffect(() => { if (!selectedCardId && cards[0]) setSelectedCardId(cards[0].windowId); }, [cards, selectedCardId]);

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
          <NavItem active={view === "index"} icon={<Clock3 />} label="Work index" onClick={() => nav("index")} />
          <NavItem active={view === "inquiries"} icon={<FolderSearch />} label="Inquiries" onClick={() => nav("inquiries")} />
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
        <span className="text-xs text-[#686a64] dark:text-[#aaa9a1]">{view === "index" ? "Today · captured on this device" : view === "inquiry" ? activeInquiry?.title || "Current inquiry" : "Dystil · local memory"}</span>
        {view === "inquiry" && <button type="button" className="inline-flex items-center gap-2 text-xs font-semibold text-[#157252] hover:text-[#0e513a] dark:text-[#56d59d] dark:hover:text-[#a5f1c8]" onClick={startInquiry}><MessageSquarePlus className="h-4 w-4" />New inquiry</button>}
      </header>

      <section className="min-h-0 overflow-hidden">
        {view === "ask" && <InquiryHome input={input} setInput={setInput} submit={submit} busy={busy} sessions={sessions} cards={cards} loadingCards={loadingCards} onOpenInquiry={openInquiry} onOpenIndex={() => nav("index")} />}
        {view === "inquiry" && <InquiryTrail turns={turns} loading={loadingSession} busy={busy} agentMessages={agentMessages} peers={peers} trailEnd={trailEnd} />}
        {view === "index" && <WorkIndex cards={cards} loading={loadingCards} selectedCard={selectedCard} onSelect={setSelectedCardId} />}
        {view === "inquiries" && <Inquiries sessions={sessions} onOpen={openInquiry} onNew={startInquiry} />}
        {view === "settings" && <SettingsPage recording={recording} toggling={toggling} onToggle={onToggleRecording} screenshots={screenshotEnabled} onScreenshot={onScreenshotChange} screenshotBusy={screenshotBusy} userName={userName} userEmail={userEmail} onLogout={onLogout} loggingOut={loggingOut} version={version} />}
      </section>

      {view === "inquiry" && <Composer input={input} setInput={setInput} busy={busy} submit={submit} mentionOpen={mentionOpen} setMentionOpen={setMentionOpen} visiblePeers={visiblePeers} choosePeer={choosePeer} />}
    </div>
  </main>;
}

function NavItem({ active, icon, label, onClick }: { active: boolean; icon: React.ReactNode; label: string; onClick: () => void }) {
  return <button type="button" onClick={onClick} className={cn("flex items-center gap-3 rounded-lg px-3 py-2.5 text-left text-[13px] transition", active ? "bg-[#dfe8df] text-[#173c2b] shadow-[inset_2px_0_0_#157252] dark:bg-[#252a27] dark:text-[#f1f0eb] dark:shadow-[inset_2px_0_0_#56d59d]" : "text-[#686a64] hover:bg-black/[.045] hover:text-[#242522] dark:text-[#aaa9a1] dark:hover:bg-white/[.06] dark:hover:text-[#f1f0eb]")}>{icon && <span className="[&_svg]:h-4 [&_svg]:w-4">{icon}</span>}{label}</button>;
}

function InquiryHome({ input, setInput, submit, busy, sessions, cards, loadingCards, onOpenInquiry, onOpenIndex }: { input: string; setInput: (value: string) => void; submit: () => Promise<void>; busy: boolean; sessions: ChatSession[]; cards: Card[]; loadingCards: boolean; onOpenInquiry: (sessionId: string) => Promise<void>; onOpenIndex: () => void }) {
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

        <div className="mt-16 max-w-[700px]">
          <div className="flex items-center justify-between border-b border-black/10 pb-3 text-[10px] font-bold uppercase tracking-[.1em] text-[#74766f] dark:border-white/10 dark:text-[#85867f]"><span>Recent capture</span><button type="button" onClick={onOpenIndex} className="normal-case tracking-normal text-[#157252] hover:underline dark:text-[#56d59d]">Open work index <ChevronRight className="inline h-3 w-3" /></button></div>
          {loadingCards ? <p className="py-5 text-sm text-[#686a64] dark:text-[#aaa9a1]"><Loader2 className="mr-2 inline h-4 w-4 animate-spin" />Reading local activity…</p> : cards.slice(0, 4).map((card) => <div key={card.windowId} className="grid grid-cols-[58px_minmax(0,1fr)_auto] gap-3 border-b border-black/10 py-3 dark:border-white/10"><time className="text-[11px] text-[#74766f] dark:text-[#85867f]">{formatTime(card.startTime)}</time><p className="truncate text-[12px] font-semibold">{card.applications[0] || "Captured activity"} · {card.title}</p><span className="text-[11px] text-[#686a64] dark:text-[#aaa9a1]">Captured</span></div>)}
          {!loadingCards && !cards.length && <p className="py-5 text-sm text-[#686a64] dark:text-[#aaa9a1]">Your captured activity will appear here.</p>}
        </div>
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

function WorkIndex({ cards, loading, selectedCard, onSelect }: { cards: Card[]; loading: boolean; selectedCard?: Card; onSelect: (id: string) => void }) {
  return <div className="grid h-full min-h-0 grid-cols-[minmax(0,1.25fr)_minmax(330px,.75fr)] overflow-hidden"><section className="overflow-auto px-8 py-10"><div className="flex items-end justify-between border-b border-black/10 pb-5 dark:border-white/10"><div><p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#157252] dark:text-[#56d59d]">Work index</p><h1 className="mt-3 font-serif text-4xl tracking-[-.035em]">Today, captured.</h1></div><span className="text-xs text-[#686a64] dark:text-[#aaa9a1]">{cards.length} moments</span></div><div className="grid grid-cols-[80px_150px_minmax(0,1fr)] gap-4 border-b border-black/10 px-3 py-3 text-[9px] font-bold uppercase tracking-[.1em] text-[#74766f] dark:border-white/10 dark:text-[#85867f]"><span>Time</span><span>Source</span><span>Captured activity</span></div>{loading ? <p className="p-5 text-sm text-[#686a64] dark:text-[#aaa9a1]"><Loader2 className="mr-2 inline h-4 w-4 animate-spin" />Reading local activity…</p> : cards.map((card) => <button type="button" key={card.windowId} onClick={() => onSelect(card.windowId)} className={cn("grid w-full grid-cols-[80px_150px_minmax(0,1fr)] gap-4 border-b border-black/10 px-3 py-4 text-left transition dark:border-white/10", selectedCard?.windowId === card.windowId ? "bg-[#dfe8df]/70 shadow-[inset_2px_0_0_#157252] dark:bg-white/[.06] dark:shadow-[inset_2px_0_0_#56d59d]" : "hover:bg-black/[.025] dark:hover:bg-white/[.035]")}><time className="text-[11px] text-[#686a64] dark:text-[#aaa9a1]">{formatTime(card.startTime)}</time><div><b className="block truncate text-xs">{card.applications[0] || "Desktop"}</b><span className="mt-1 block truncate text-[11px] text-[#686a64] dark:text-[#aaa9a1]">{card.status}</span></div><div><b className="block truncate font-serif text-lg font-normal">{card.title}</b><span className="mt-1 block truncate text-xs text-[#686a64] dark:text-[#aaa9a1]">{card.summary || "Captured context available for retrieval."}</span></div></button>)}{!loading && !cards.length && <p className="p-5 text-sm text-[#686a64] dark:text-[#aaa9a1]">Captured activity will appear here as you work.</p>}</section><aside className="overflow-auto border-l border-black/10 bg-[#eeece5]/70 px-8 py-10 dark:border-white/10 dark:bg-[#1d1f1e]"><p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#157252] dark:text-[#56d59d]">Selected moment</p>{selectedCard ? <><h2 className="mt-4 font-serif text-3xl font-normal tracking-[-.03em]">{selectedCard.title}</h2><p className="mt-4 text-sm leading-6 text-[#686a64] dark:text-[#aaa9a1]">{selectedCard.summary || "Captured context is available for retrieval."}</p><div className="mt-7 border-y border-black/10 py-4 text-xs leading-6 text-[#686a64] dark:border-white/10 dark:text-[#aaa9a1]"><b className="text-[#242522] dark:text-[#f1f0eb]">Captured context.</b> This record is available locally. Any generated conclusion should remain visibly grounded in its sources.</div><p className="mt-7 text-[10px] font-bold uppercase tracking-[.1em] text-[#74766f] dark:text-[#85867f]">Available sources</p><div className="mt-3 divide-y divide-black/10 border-y border-black/10 dark:divide-white/10 dark:border-white/10">{(selectedCard.applications.length ? selectedCard.applications : ["Desktop activity"]).map((application) => <div key={application} className="flex items-center gap-3 py-3"><FileText className="h-4 w-4 text-[#157252] dark:text-[#56d59d]" /><span className="text-xs">{application}</span></div>)}</div></> : <p className="mt-5 text-sm text-[#686a64] dark:text-[#aaa9a1]">Choose a captured moment to inspect its context.</p>}</aside></div>;
}

function Inquiries({ sessions, onOpen, onNew }: { sessions: ChatSession[]; onOpen: (id: string) => Promise<void>; onNew: () => void }) { return <div className="h-full overflow-auto px-12 py-12"><div className="max-w-4xl"><div className="flex items-end justify-between border-b border-black/10 pb-5 dark:border-white/10"><div><p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#157252] dark:text-[#56d59d]">Inquiries</p><h1 className="mt-3 font-serif text-4xl tracking-[-.035em]">Questions worth returning to.</h1></div><button type="button" onClick={onNew} className="inline-flex items-center gap-2 text-xs font-semibold text-[#157252] dark:text-[#56d59d]"><MessageSquarePlus className="h-4 w-4" />New inquiry</button></div><div className="divide-y divide-black/10 border-b border-black/10 dark:divide-white/10 dark:border-white/10">{sessions.map((session) => <button type="button" key={session.id} onClick={() => void onOpen(session.id)} className="grid w-full grid-cols-[minmax(0,1fr)_auto] gap-5 py-5 text-left hover:bg-black/[.025] dark:hover:bg-white/[.035]"><div><b className="block truncate font-serif text-xl font-normal">{session.title}</b><span className="mt-2 block text-xs text-[#686a64] dark:text-[#aaa9a1]">Open the inquiry to continue asking against its existing context.</span></div><time className="self-center text-xs text-[#74766f] dark:text-[#85867f]">{formatDate(session.updatedAt)}</time></button>)}{!sessions.length && <div className="py-12 text-sm text-[#686a64] dark:text-[#aaa9a1]">No saved inquiries yet. Your related questions will appear here after the first answer is stored.</div>}</div></div></div>; }

function Composer({ input, setInput, busy, submit, mentionOpen, setMentionOpen, visiblePeers, choosePeer }: { input: string; setInput: (value: string) => void; busy: boolean; submit: () => Promise<void>; mentionOpen: boolean; setMentionOpen: (value: boolean) => void; visiblePeers: Peer[]; choosePeer: (peer: Peer) => void }) { return <div className="absolute bottom-7 left-[196px] right-0 z-10 mx-auto max-w-[900px] px-8"><div className="relative border border-black/15 bg-[#fffefa] p-2 shadow-[0_16px_40px_rgba(20,24,22,.16)] dark:border-white/15 dark:bg-[#252725] dark:shadow-[0_16px_40px_rgba(0,0,0,.28)]"><input value={input} onChange={(event) => { const value = event.target.value; setInput(value); if (value.includes("@")) setMentionOpen(true); }} onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); void submit(); } }} placeholder="Continue this inquiry…" className="h-11 w-full bg-transparent px-3 pr-12 text-sm outline-none placeholder:text-[#85867f]" /><button type="button" onClick={() => void submit()} disabled={busy || !input.trim()} className="absolute right-3 top-3 grid h-9 w-9 place-items-center rounded-md bg-[#157252] text-white disabled:opacity-35 dark:bg-[#56d59d] dark:text-[#151616]">{busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}</button>{mentionOpen && <div className="absolute bottom-[62px] left-0 w-full max-w-[390px] border border-black/15 bg-[#fffefa] p-2 shadow-xl dark:border-white/15 dark:bg-[#252725]"><p className="px-2 py-1 text-[10px] font-bold uppercase tracking-[.1em] text-[#74766f] dark:text-[#85867f]">Ask a teammate’s Dystil</p>{visiblePeers.length ? visiblePeers.map((peer) => <button type="button" key={peer.userId} onClick={() => choosePeer(peer)} className="flex w-full items-center gap-3 px-2 py-2 text-left hover:bg-black/[.04] dark:hover:bg-white/[.06]"><span className="grid h-7 w-7 place-items-center rounded-full bg-[#dfe8df] text-[10px] font-semibold text-[#157252] dark:bg-[#173a30] dark:text-[#56d59d]">{(peer.displayName || peer.email).slice(0, 2).toUpperCase()}</span><span className="min-w-0 flex-1"><b className="block text-xs">{peer.displayName || peer.email.split("@")[0]}</b><span className="block truncate text-[11px] text-[#686a64] dark:text-[#aaa9a1]">{peer.email}</span></span></button>) : <p className="p-2 text-xs text-[#686a64] dark:text-[#aaa9a1]">No compatible teammates are available yet.</p>}</div>}</div><p className="flex justify-between px-1 pt-2 text-[11px] text-[#686a64] dark:text-[#aaa9a1]"><span><AtSign className="mr-1 inline h-3 w-3" />Ask a teammate</span><span>Enter to continue inquiry</span></p></div>; }

function LocalAnswer({ chat }: { chat: Chat }) {
  if (chat.status === "pending") return <div className="border-l border-[#157252] py-2 pl-4 text-sm text-[#686a64] dark:border-[#56d59d] dark:text-[#aaa9a1]"><Loader2 className="mr-2 inline h-4 w-4 animate-spin" />Searching the relevant local work…</div>;
  const provider = chat.provider === "byok" ? "Your personal AI" : chat.provider === "claude" ? "Claude Code" : chat.provider === "codex" ? "Codex" : chat.provider;
  const answer = chat.status === "failed" && chat.historical
    ? "This earlier attempt did not complete. The rest of this inquiry and its saved sources are still available."
    : chat.answer || "Dystil could not generate an answer from this local context.";
  return <div className="border-l border-black/15 py-1 pl-4 dark:border-white/15"><p className={cn("text-sm leading-6", chat.status === "failed" && "text-destructive")}>{answer}</p>{chat.citations?.length ? <div className="mt-4 border-t border-black/10 pt-3 text-xs text-[#686a64] dark:border-white/10 dark:text-[#aaa9a1]"><b className="mb-1 block text-[10px] font-bold uppercase tracking-[.1em]">Available sources</b>{chat.citations.map((item) => <p key={`${item.label}-${item.localDate}`}>• {item.label}{item.localDate ? ` — ${item.localDate}` : ""}</p>)}</div> : null}{provider ? <p className="mt-3 text-[10px] text-[#74766f] dark:text-[#85867f]">Answered by {provider}{chat.model ? ` · ${chat.model}` : ""}</p> : null}</div>;
}

function TeamAnswer({ messages, peers }: { messages: AgentMessage[]; peers: Peer[] }) { const answer = messages.find((message) => message.kind === "response"); if (!answer) return <div className="border-l border-[#157252] py-2 pl-4 text-sm text-[#686a64] dark:border-[#56d59d] dark:text-[#aaa9a1]"><Loader2 className="mr-2 inline h-4 w-4 animate-spin" />Waiting for a teammate’s Dystil…</div>; const name = peers.find((peer) => peer.userId === answer.peerUserId)?.displayName || "Teammate"; return <div className="border-l border-black/15 py-1 pl-4 dark:border-white/15"><p className="text-sm leading-6">{answer.text}</p><p className="mt-3 text-[10px] font-bold uppercase tracking-[.1em] text-[#74766f] dark:text-[#85867f]">Evidence from {name}’s local work</p>{answer.evidence.map((item) => <p key={item.label} className="mt-1 text-xs text-[#686a64] dark:text-[#aaa9a1]">• {item.label} — {item.localDate}</p>)}</div>; }

function SettingsPage({ recording, toggling, onToggle, screenshots, onScreenshot, screenshotBusy, userName, userEmail, onLogout, loggingOut, version }: { recording: boolean; toggling: boolean; onToggle: () => void; screenshots: boolean; onScreenshot: (value: boolean) => void; screenshotBusy: boolean; userName: string; userEmail: string; onLogout: () => void; loggingOut: boolean; version: string }) { return <div className="h-full overflow-auto px-12 py-12"><div className="max-w-3xl"><p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#157252] dark:text-[#56d59d]">Local memory</p><h1 className="mt-3 font-serif text-4xl tracking-[-.035em]">Capture settings</h1><p className="mt-3 text-sm text-[#686a64] dark:text-[#aaa9a1]">Control what Dystil can retain on this device.</p><div className="mt-10 divide-y divide-black/10 border-y border-black/10 dark:divide-white/10 dark:border-white/10"><SettingRow label="Local capture" description={recording ? "Dystil is capturing locally." : "Capture is paused."} action={<Button onClick={onToggle} disabled={toggling} variant="outline" className="rounded-lg">{toggling ? <Loader2 className="h-4 w-4 animate-spin" /> : recording ? <Pause className="h-4 w-4" /> : <Play className="h-4 w-4" />}{recording ? "Pause" : "Resume"}</Button>} /><SettingRow label="Capture screenshots" description="Off by default. Accessibility-only capture remains available." action={screenshotBusy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Switch aria-label="Capture screenshots" checked={screenshots} onCheckedChange={onScreenshot} />} /><SettingRow label="Account" description={`${userName} · ${userEmail}`} action={<Button variant="outline" onClick={onLogout} disabled={loggingOut} className="rounded-lg">{loggingOut ? <Loader2 className="h-4 w-4 animate-spin" /> : "Sign out"}</Button>} /></div><ManagedProviderSettings /><ByokSettings /><ExternalMcpSettings /><ExperimentalLocalProcessingSettings />{version && <p className="mt-5 text-xs text-[#74766f] dark:text-[#85867f]">Dystil v{version}</p>}</div></div>; }

type ManagedProvider = "codex" | "claude";
type ManagedProviderStatus = { provider: ManagedProvider; state: string; installedVersion: string | null; authenticated: boolean | null; detail: string | null };
type ManagedProviderModel = { id: string; displayName: string; description: string; isDefault: boolean };

function ManagedProviderSettings() {
  const [statuses, setStatuses] = useState<Record<ManagedProvider, ManagedProviderStatus | null>>({ codex: null, claude: null });
  const [models, setModels] = useState<Record<ManagedProvider, ManagedProviderModel[]>>({ codex: [], claude: [] });
  const [providerModels, setProviderModels] = useState<Record<ManagedProvider, string>>({ codex: "default", claude: "default" });
  const [modelsLoading, setModelsLoading] = useState(false);
  const [selected, setSelected] = useState<ManagedProvider>("codex");
  const [byokActive, setByokActive] = useState(false);
  const [busy, setBusy] = useState<ManagedProvider | null>(null);
  const [message, setMessage] = useState("");
  const [claudeCodeRequired, setClaudeCodeRequired] = useState(false);
  const [claudeAuthorizationCode, setClaudeAuthorizationCode] = useState("");
  const refresh = async () => {
    try {
      const [codex, claude, preference, profiles] = await Promise.all([
        invoke<ManagedProviderStatus>("ai_provider_status", { provider: "codex" }),
        invoke<ManagedProviderStatus>("ai_provider_status", { provider: "claude" }),
        invoke<{ provider: ManagedProvider; model: string }>("agent_get_preferences"),
        invoke<ByokProfile[]>("byok_list_profiles"),
      ]);
      setStatuses({ codex, claude }); setSelected(preference.provider); setByokActive(profiles.length > 0);
      setModelsLoading(true);
      const [codexModels, claudeModels] = await Promise.all([
        codex.state === "ready" ? invoke<ManagedProviderModel[]>("ai_provider_models", { provider: "codex" }).catch(() => []) : [],
        claude.state === "ready" ? invoke<ManagedProviderModel[]>("ai_provider_models", { provider: "claude" }).catch(() => []) : [],
      ]);
      setModels({ codex: codexModels, claude: claudeModels });
      setProviderModels((current) => ({
        codex: preference.provider === "codex"
          ? preference.model
          : modelAvailable(current.codex, codexModels),
        claude: preference.provider === "claude"
          ? preference.model
          : modelAvailable(current.claude, claudeModels),
      }));
      setModelsLoading(false);
    } catch { setMessage("Could not read AI connection status."); }
  };
  useEffect(() => {
    void refresh();
    window.addEventListener("dystil-byok-changed", refresh);
    let unlisten: (() => void) | undefined;
    listen("ai-provider-login-updated", () => void refresh()).then((dispose) => { unlisten = dispose; }).catch(() => undefined);
    return () => { window.removeEventListener("dystil-byok-changed", refresh); unlisten?.(); };
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
    try { await invoke("agent_set_preferences", { provider, model: nextModel }); setSelected(provider); setProviderModels((current) => ({ ...current, [provider]: nextModel })); setMessage(`${provider === "codex" ? "Codex" : "Claude Code"} will answer inquiries when Personal AI is not active.`); }
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
      await invoke("agent_set_preferences", { provider, model: nextModel });
      setMessage(`${models[provider].find((item) => item.id === nextModel)?.displayName || nextModel} will be used for managed chat.`);
    } catch (error) { setMessage(error instanceof Error ? error.message : String(error || "Could not save the managed model.")); }
    finally { setBusy(null); }
  };
  return <section className="mt-10 border-t border-black/10 pt-8 dark:border-white/10">
    <p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#157252] dark:text-[#56d59d]">Managed AI</p>
    <h2 className="mt-2 font-serif text-2xl font-normal tracking-[-.02em]">Connect Codex or Claude Code</h2>
    <p className="mt-2 max-w-[64ch] text-xs leading-5 text-[#686a64] dark:text-[#aaa9a1]">Dystil installs the official CLI privately, opens its own browser sign-in, and gives that process only Dystil’s bounded local retrieval tools for an inquiry. Dystil never receives the provider account token.</p>
    {byokActive && <p className="mt-4 border-y border-black/10 py-3 text-xs leading-5 text-[#686a64] dark:border-white/10 dark:text-[#aaa9a1]">Personal AI is active, so it currently takes priority for chat and work cards. Remove that profile below to use a managed connection.</p>}
    <div className="mt-5 divide-y divide-black/10 border-y border-black/10 dark:divide-white/10 dark:border-white/10">{(["codex", "claude"] as const).map((provider) => {
      const status = statuses[provider];
      const name = provider === "codex" ? "Codex" : "Claude Code";
      const ready = status?.state === "ready" && status.authenticated;
      const repair = status?.state === "repairRequired";
      const active = selected === provider && !byokActive;
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
          <div className="flex items-center gap-2">{ready ? <><Button type="button" variant="outline" className="rounded-lg" disabled={busy !== null} onClick={() => void test(provider)}>{busy === provider ? <Loader2 className="h-4 w-4 animate-spin" /> : "Check"}</Button><Button type="button" className="rounded-lg" disabled={busy !== null || byokActive || active} onClick={() => void select(provider)}>{active ? "In use" : "Use for chat"}</Button></> : <Button type="button" className="rounded-lg" disabled={busy !== null} onClick={() => void connect(provider)}>{busy === provider ? <Loader2 className="h-4 w-4 animate-spin" /> : status?.state === "ready" ? "Sign in" : repair ? "Repair & sign in" : "Install & sign in"}</Button>}</div>
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

type ByokProfile = { id: string; endpoint: string; chatModel: string; workCardModel: string; credentialPresent: boolean };

function ByokSettings() {
  const [profiles, setProfiles] = useState<ByokProfile[]>([]);
  const [endpoint, setEndpoint] = useState("https://api.openai.com");
  const [chatModel, setChatModel] = useState("gpt-5.6-luna");
  const [workCardModel, setWorkCardModel] = useState("gpt-5.6-luna");
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const refresh = async () => {
    try { setProfiles(await invoke<ByokProfile[]>("byok_list_profiles")); }
    catch { setMessage("Could not read the local AI profile."); }
  };
  useEffect(() => { void refresh(); }, []);
  const save = async (event: React.FormEvent) => {
    event.preventDefault(); if (!apiKey.trim() || busy) return;
    setBusy(true); setMessage("");
    try {
      await invoke("byok_save_profile", { endpoint, chatModel, workCardModel, apiKey });
      setApiKey(""); setMessage("Saved locally. The key is held by your operating system credential store."); await refresh(); window.dispatchEvent(new Event("dystil-byok-changed"));
    } catch (error) { setMessage(error instanceof Error ? error.message : "Could not save the AI profile."); }
    finally { setBusy(false); }
  };
  const remove = async (id: string) => {
    setBusy(true); setMessage("");
    try { await invoke("byok_delete_profile", { profileId: id }); await refresh(); window.dispatchEvent(new Event("dystil-byok-changed")); }
    catch (error) { setMessage(error instanceof Error ? error.message : "Could not remove the AI profile."); }
    finally { setBusy(false); }
  };
  const active = profiles[0];
  return <section className="mt-10 border-t border-black/10 pt-8 dark:border-white/10">
    <p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#157252] dark:text-[#56d59d]">Personal AI</p>
    <h2 className="mt-2 font-serif text-2xl font-normal tracking-[-.02em]">Use your own OpenAI-compatible key</h2>
    <p className="mt-2 max-w-[64ch] text-xs leading-5 text-[#686a64] dark:text-[#aaa9a1]">Dystil uses this profile for work-card generation and local inquiries. Your key never appears in Dystil again after saving.</p>
    {active ? <div className="mt-5 flex items-center justify-between gap-4 border-y border-black/10 py-4 dark:border-white/10"><p className="min-w-0 text-xs"><b className="block truncate">{active.chatModel}</b><span className="block truncate text-[#686a64] dark:text-[#aaa9a1]">{active.endpoint} · {active.credentialPresent ? "Key available" : "Key unavailable"}</span></p><Button type="button" variant="outline" className="rounded-lg" disabled={busy} onClick={() => void remove(active.id)}>Remove</Button></div> : <form onSubmit={save} className="mt-5 grid gap-3 border-y border-black/10 py-5 dark:border-white/10"><label className="grid gap-1 text-xs font-semibold">Endpoint<input value={endpoint} onChange={(event) => setEndpoint(event.target.value)} required className="h-9 rounded-md border border-black/15 bg-transparent px-2 font-normal outline-none focus:border-[#157252] dark:border-white/15 dark:focus:border-[#56d59d]" /></label><div className="grid grid-cols-2 gap-3"><label className="grid gap-1 text-xs font-semibold">Chat model<input value={chatModel} onChange={(event) => setChatModel(event.target.value)} required className="h-9 rounded-md border border-black/15 bg-transparent px-2 font-normal outline-none focus:border-[#157252] dark:border-white/15 dark:focus:border-[#56d59d]" /></label><label className="grid gap-1 text-xs font-semibold">Work-card model<input value={workCardModel} onChange={(event) => setWorkCardModel(event.target.value)} required className="h-9 rounded-md border border-black/15 bg-transparent px-2 font-normal outline-none focus:border-[#157252] dark:border-white/15 dark:focus:border-[#56d59d]" /></label></div><label className="grid gap-1 text-xs font-semibold">API key<input type="password" autoComplete="off" value={apiKey} onChange={(event) => setApiKey(event.target.value)} required className="h-9 rounded-md border border-black/15 bg-transparent px-2 font-normal outline-none focus:border-[#157252] dark:border-white/15 dark:focus:border-[#56d59d]" /></label><div><Button type="submit" disabled={busy || !apiKey.trim()} className="rounded-lg">{busy ? <Loader2 className="h-4 w-4 animate-spin" /> : null}Save personal AI</Button></div></form>}
    {message && <p role="status" className="mt-3 text-xs leading-5 text-[#686a64] dark:text-[#aaa9a1]">{message}</p>}
  </section>;
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
      <span>I understand that the selected external AI client will be able to read Dystil’s sanitized activity and work-card data. Screenshots, raw accessibility trees, writes, shell access, and arbitrary database access are not shared. Codex setup also adds a marked Dystil preference to its existing global guidance; it never replaces your instructions.</span>
    </label>
    <div className="mt-4 flex flex-wrap gap-3"><Button type="button" className="rounded-lg" disabled={!consented || busy !== null} onClick={() => void add("codex")}>{busy === "codex" && <Loader2 className="h-4 w-4 animate-spin" />}Add Dystil to Codex</Button><Button type="button" variant="outline" className="rounded-lg" disabled={!consented || busy !== null} onClick={() => void add("claude")}>{busy === "claude" && <Loader2 className="h-4 w-4 animate-spin" />}Add Dystil to Claude Code</Button></div>
    <p className="mt-3 text-[11px] leading-5 text-[#74766f] dark:text-[#85867f]">Requires the selected client’s own CLI to be installed on this computer. Dystil updates only that client’s MCP configuration.</p>
    {message && <p role="status" className="mt-3 text-xs leading-5 text-[#686a64] dark:text-[#aaa9a1]">{message}</p>}
  </section>;
}

function ExperimentalLocalProcessingSettings() {
  const [enabled, setEnabled] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  useEffect(() => { void invoke<{ enabled: boolean }>("get_local_processing_status").then((status) => setEnabled(status.enabled)).catch(() => setMessage("Could not read local processing status.")); }, []);
  const change = async (next: boolean) => {
    setBusy(true); setMessage("");
    try {
      const status = await invoke<{ enabled: boolean }>("set_local_processing_enabled", { enabled: next });
      setEnabled(status.enabled);
      setMessage(next ? "Dystil is preparing the local model in the background. It will generate activity summaries when ready." : "Managed AI will generate future activity summaries again.");
    } catch (error) { setMessage(error instanceof Error ? error.message : "Could not change local processing."); }
    finally { setBusy(false); }
  };
  return <section className="mt-10 border-t border-black/10 pt-8 dark:border-white/10">
    <p className="text-[10px] font-bold uppercase tracking-[.12em] text-[#157252] dark:text-[#56d59d]">Experimental</p>
    <div className="mt-2 flex items-start justify-between gap-6"><div><h2 className="font-serif text-2xl font-normal tracking-[-.02em]">Generate activity summaries locally</h2><p className="mt-2 max-w-[64ch] text-xs leading-5 text-[#686a64] dark:text-[#aaa9a1]">Downloads a roughly 1.3 GB model and uses it instead of Codex or Claude Code for future activity summaries. Its quality can be noticeably worse or incomplete. Ask Your Work still uses your connected provider.</p></div><Switch aria-label="Enable experimental local activity processing" checked={enabled} disabled={busy} onCheckedChange={(next) => void change(next)} /></div>
    {message && <p role="status" className="mt-3 text-xs leading-5 text-[#686a64] dark:text-[#aaa9a1]">{busy && <Loader2 className="mr-1 inline h-3.5 w-3.5 animate-spin" />}{message}</p>}
  </section>;
}
function SettingRow({ label, description, action }: { label: string; description: string; action: React.ReactNode }) { return <div className="flex items-center gap-6 py-5"><div className="min-w-0 flex-1"><b className="text-sm">{label}</b><p className="mt-1 text-xs text-[#686a64] dark:text-[#aaa9a1]">{description}</p></div>{action}</div>; }
function formatTime(value: string) { const date = new Date(value); return Number.isNaN(date.getTime()) ? "—" : date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }); }
function formatDate(value: string) { const date = new Date(`${value}Z`); return Number.isNaN(date.getTime()) ? "Recently" : date.toLocaleDateString([], { month: "short", day: "numeric" }); }

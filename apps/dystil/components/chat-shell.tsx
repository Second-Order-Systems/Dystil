"use client";

import { useState } from "react";
import { usePlatform } from "@/lib/hooks/use-platform";

type Peer = { userId: string; displayName: string | null; email: string; agentStatus: string };
type AgentMessage = { messageId: string; peerUserId: string; direction: string; kind: string; localStatus: string; text: string; evidence: Array<{ label: string; localDate: string }> };

export type ChatSession = { id: string; title: string; updatedAt: string };
export type Chat = {
  id: string;
  conversationId: string;
  question: string;
  mode: "local" | "team";
  answer?: string | null;
  status?: "pending" | "complete" | "failed";
  citations?: Array<{ label: string; localDate: string }>;
  provider?: string | null;
  model?: string | null;
  elapsedMs?: number | null;
  historical?: boolean;
};

type Props = {
  userName: string;
  userEmail: string;
  recording: boolean;
  toggling: boolean;
  onToggleRecording: () => void;
  screenshotEnabled: boolean;
  onScreenshotChange: (value: boolean) => void;
  screenshotBusy: boolean;
  peers: Peer[];
  agentMessages: AgentMessage[];
  sessions: ChatSession[];
  onLoadSession: (sessionId: string) => Promise<Chat[]>;
  onSendLocal: (sessionId: string, question: string) => Promise<Chat>;
  onAskPeer: (peerId: string, question: string) => Promise<void>;
  onLogout: () => void;
  loggingOut: boolean;
  version: string;
};

type View = "worth-fixing" | "ready" | "ask" | "privacy" | "settings";
type SettingsTab = "What Dystil can see" | "Model and cost" | "When it runs" | "Storage" | "Notifications" | "Invite your team" | "About";

const signals = [
  {
    title: "The same work, over and over",
    description: "You do it the same way every time. If nothing about it changes, it does not need you.",
  },
  {
    title: "Work that arrives on a schedule",
    description: <>The Monday report, the month-end close. Most of that time is setup and waiting, and it can be done before you sit<br />down.</>,
  },
  {
    title: "Work where you make the call",
    description: "The judgement has to be yours. Rebuilding the same groundwork before every one of them does not.",
  },
  {
    title: "Work that could come out better",
    description: "The report, the reply, the summary. Done to the standard you would want if you had the time.",
  },
  {
    title: "What you would do if you had the time",
    description: "The prep before the call, the check before the decision. Skipped because the day is full, not because it does not matter.",
  },
];

const readyFixes = [
  {
    id: "monday-update",
    title: "Draft the Monday update",
    description: "Pull the week’s completed work into the same update format you use every Monday.",
    evidence: "Seen across 6 Mondays · Excel and Outlook",
    steps: ["Collect completed items from the weekly tracker", "Draft the update in your usual section order", "Leave it ready for your review before sending"],
  },
  {
    id: "review-context",
    title: "Prepare context before pipeline reviews",
    description: "Gather recent account notes and open decisions before the recurring review call.",
    evidence: "Seen across 4 review cycles · HubSpot and Slack",
    steps: ["Find accounts on the review agenda", "Collect their latest notes and unresolved decisions", "Prepare one concise brief for the call"],
  },
  {
    id: "client-check",
    title: "Run the client-report check",
    description: "Apply the same final checks you make before a client report goes out.",
    evidence: "Seen across 9 reports · Google Docs and Outlook",
    steps: ["Check dates, totals, and client names", "Flag missing sections or unresolved comments", "Return the report with issues clearly marked"],
  },
];

export function ChatShell(props: Props) {
  const [view, setView] = useState<View>("worth-fixing");
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("What Dystil can see");
  const { isMac } = usePlatform();
  const shellClassName = `relative h-dvh min-h-[640px] min-w-[760px] overflow-hidden bg-[#fdfdfc] text-[#0d0e0d] ${isMac ? "pt-[38px]" : ""}`;

  if (view === "settings") {
    return (
      <main className={`${shellClassName} text-[#1a1c20]`}>
        {isMac && <div data-tauri-drag-region className="absolute inset-x-0 top-0 h-[38px] border-b border-[#e4e2dc] bg-[#f5f3ed]" aria-hidden="true" />}
        <div className="h-full overflow-hidden bg-[#f8f8f7]">
          <SettingsWorkspace {...props} initialTab={settingsTab} onBack={() => setView("worth-fixing")} />
        </div>
      </main>
    );
  }

  return (
    <main className={shellClassName}>
      {isMac && <div data-tauri-drag-region className="absolute inset-x-0 top-0 h-[38px] border-b border-[#e4e2dc] bg-[#f5f3ed]" aria-hidden="true" />}
      <div className="grid h-full grid-cols-[268px_minmax(0,1fr)] overflow-hidden bg-[#fdfdfc]">
        <Sidebar
          view={view}
          setView={setView}
          openSettings={(tab) => {
            setSettingsTab(tab);
            setView("settings");
          }}
        />
        <section className="min-h-0 overflow-y-auto px-[47px] pb-[44px] pt-[45px]">
          {view === "worth-fixing" && <WorthFixing onAsk={() => setView("ask")} />}
          {view === "ready" && <ReadyToUse onAsk={() => setView("ask")} />}
          {view === "ask" && <AskForFix />}
          {view === "privacy" && <Privacy onOpenSettings={() => setView("settings")} />}
        </section>
      </div>
    </main>
  );
}

function Sidebar({ view, setView, openSettings }: { view: View; setView: (view: View) => void; openSettings: (tab: SettingsTab) => void }) {
  return (
    <aside className="flex min-h-0 flex-col border-r border-[#dfded9] bg-white py-[27px]">
      <button
        type="button"
        className="mb-[28px] w-fit px-[26px] text-left text-[18px] font-semibold tracking-[0.28em] text-[#07110e] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-[#078260]"
        onClick={() => setView("worth-fixing")}
        aria-label="Dystil home"
      >
        D<span className="text-[#087d63]">Y</span>STIL
      </button>

      <nav aria-label="Primary navigation">
        <SidebarButton active={view === "worth-fixing"} onClick={() => setView("worth-fixing")}>Worth fixing</SidebarButton>
        <SidebarButton active={view === "ready"} onClick={() => setView("ready")}>Ready to use</SidebarButton>
        <SidebarButton active={view === "ask"} onClick={() => setView("ask")}>Ask for a fix</SidebarButton>
      </nav>

      <div className="mx-[26px] mt-auto border-t border-[#e7e5df] pt-[21px]">
        <div className="mb-[29px]">
          <p className="flex items-center gap-[10px] text-[18px] leading-[1.25] text-[#42464a]">
            <span className="h-[9px] w-[9px] shrink-0 rounded-full bg-[#12a77a]" aria-hidden="true" />
            Watching
          </p>
          <p className="mt-[7px] max-w-[190px] text-[16px] leading-[1.45] text-[#9398a0]">Nothing has left this computer. It cannot.</p>
        </div>
        <nav className="grid gap-[11px]" aria-label="Secondary navigation">
          <FooterLink active={view === "privacy"} onClick={() => setView("privacy")}>What stays on this<br />computer</FooterLink>
          <FooterLink active={false} onClick={() => openSettings("Invite your team")}>Invite your team</FooterLink>
          <FooterLink active={view === "settings"} onClick={() => openSettings("What Dystil can see")}>Settings</FooterLink>
        </nav>
      </div>
    </aside>
  );
}

function SidebarButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`relative block h-[50px] w-full px-[28px] text-left text-[20px] transition-colors focus-visible:z-10 focus-visible:outline-none focus-visible:shadow-[inset_0_0_0_1px_#078260] ${active ? "bg-[#def3ed] text-[#006a51]" : "text-[#42464a] hover:bg-[#f5f7f5]"}`}
    >
      {active && <span className="absolute inset-y-0 left-0 w-[2px] bg-[#0aa275]" aria-hidden="true" />}
      {children}
    </button>
  );
}

function FooterLink({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button type="button" onClick={onClick} className={`w-full text-left text-[17px] leading-[1.25] hover:text-[#006a51] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#078260] ${active ? "font-medium text-[#006a51]" : "text-[#42464a]"}`}>
      {children}
    </button>
  );
}

function PageHeading({ title, description }: { title: string; description: React.ReactNode }) {
  return (
    <header>
      <h1 className="text-balance text-[31px] font-normal leading-[1.22] tracking-[-0.035em] text-black">{title}</h1>
      <p className="mt-[20px] max-w-[780px] text-[20px] leading-[1.75] text-[#4f5660]">{description}</p>
    </header>
  );
}

function WorthFixing({ onAsk }: { onAsk: () => void }) {
  return (
    <div className="mx-auto max-w-[1124px]">
      <PageHeading
        title="Dystil has started reading how you work."
        description={<>It will let you know the moment it finds something that could save you time or<br className="hidden xl:block" /> make the work better.</>}
      />

      <section className="mt-[40px]">
        <h2 className="text-[22px] font-normal leading-none text-black">What it is looking for</h2>
        <div className="mt-[22px] grid gap-[14px]">
          {signals.map((signal, index) => (
            <article key={signal.title} className="rounded-[14px] border border-[#deddd8] bg-white px-[25px] py-[21px]">
              <h3 className="text-[22px] font-normal leading-[1.25] text-black">{signal.title}</h3>
              <p className={`mt-[6px] text-[19px] leading-[1.63] tracking-[-0.012em] text-[#505761] ${index === signals.length - 1 ? "xl:whitespace-nowrap" : ""}`}>{signal.description}</p>
            </article>
          ))}
        </div>
      </section>

      <section className="mt-[38px] rounded-[14px] bg-[#f5fbf8] px-[29px] py-[28px]">
        <h2 className="text-[21px] font-normal leading-[1.3] text-black">Something already annoying you?</h2>
        <p className="mt-[7px] max-w-[780px] text-[19px] leading-[1.55] text-[#505761]">Dystil will keep finding things on its own either way. Tell it what annoys you most and<br className="hidden xl:block" /> that gets looked at first.</p>
        <button type="button" onClick={onAsk} className="mt-[22px] h-[52px] rounded-[13px] bg-[#087b5f] px-[26px] text-[19px] font-medium text-white transition-colors hover:bg-[#06634d] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-3 focus-visible:outline-[#087b5f]">Ask for a fix</button>
      </section>
    </div>
  );
}

function ReadyToUse({ onAsk }: { onAsk: () => void }) {
  const [expanded, setExpanded] = useState<string | null>(readyFixes[0].id);
  const [approved, setApproved] = useState<string[]>([]);
  return (
    <div className="mx-auto max-w-[1124px]">
      <PageHeading title="Three fixes are ready." description="Dystil found repeatable work it can prepare for you. Nothing runs until you review and approve it." />
      <section className="mt-[38px]">
        <div className="border-b border-[#dad8d2] pb-[12px]">
          <h2 className="text-[20px] font-medium text-black">Ready now</h2>
        </div>
        <div className="divide-y divide-[#e1dfd9] border-b border-[#dad8d2]">
          {readyFixes.map((fix) => {
            const isExpanded = expanded === fix.id;
            const isApproved = approved.includes(fix.id);
            return <article key={fix.id} className="py-[20px]">
              <div className="grid grid-cols-[108px_minmax(0,1fr)_auto] items-start gap-[22px]">
                <p className={`mt-[3px] flex items-center gap-[8px] text-[13px] font-medium ${isApproved ? "text-[#6c726f]" : "text-[#087b5f]"}`}><span className={`h-[7px] w-[7px] rounded-full ${isApproved ? "bg-[#8f9692]" : "bg-[#12a77a]"}`} />{isApproved ? "Approved" : "Fix ready"}</p>
                <div>
                  <h3 className="text-[20px] font-medium leading-[1.3] text-[#151716]">{fix.title}</h3>
                  <p className="mt-[5px] text-[16px] leading-[1.5] text-[#596069]">{fix.description}</p>
                  <p className="mt-[9px] text-[13px] text-[#8a8f95]">{fix.evidence}</p>
                </div>
                <button type="button" onClick={() => setExpanded(isExpanded ? null : fix.id)} className="mt-[1px] min-w-[86px] rounded-[9px] border border-[#d4d2cc] bg-white px-[14px] py-[9px] text-[14px] font-medium text-[#3d4247] transition-colors hover:border-[#8db7a8] hover:text-[#087b5f] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#087b5f]">{isExpanded ? "Close" : "Review"}</button>
              </div>
              {isExpanded && <div className="ml-[130px] mt-[17px] rounded-[12px] bg-[#f5faf7] px-[18px] py-[16px]">
                <p className="text-[14px] font-medium text-[#22302b]">What Dystil will do</p>
                <ol className="mt-[10px] grid gap-[7px]">{fix.steps.map((step, index) => <li key={step} className="flex gap-[10px] text-[14px] leading-[1.5] text-[#5c6460]"><span className="text-[#0f6e56]">{index + 1}.</span><span>{step}</span></li>)}</ol>
                <div className="mt-[15px] flex items-center gap-[14px]">
                  <button type="button" disabled={isApproved} onClick={() => setApproved((current) => current.includes(fix.id) ? current : [...current, fix.id])} className="rounded-[9px] bg-[#087b5f] px-[16px] py-[9px] text-[14px] font-medium text-white hover:bg-[#06634d] disabled:bg-[#d7ddd9] disabled:text-[#78817c]">{isApproved ? "Approved" : "Approve this fix"}</button>
                  <button type="button" onClick={() => setExpanded(null)} className="text-[14px] text-[#6a706d] hover:text-[#222624]">Not now</button>
                </div>
              </div>}
            </article>;
          })}
        </div>
      </section>
      <button type="button" onClick={onAsk} className="mt-[24px] text-[15px] font-medium text-[#087b5f] hover:text-[#055d49]">Want Dystil to look at something else? Ask for a fix →</button>
    </div>
  );
}

function AskForFix() {
  const [request, setRequest] = useState("");
  const [sent, setSent] = useState(false);
  const suggestions = [
    "I rebuild the same report every week.",
    "I copy the same information between apps.",
    "I repeat the same checks before work goes out.",
  ];
  const submitRequest = () => {
    if (request.trim()) setSent(true);
  };
  return (
    <div className="mx-auto max-w-[960px] pt-[7px]">
      <header className="max-w-[720px]">
        <h1 className="text-balance text-[34px] font-normal leading-[1.2] tracking-[-0.035em] text-black">Ask Dystil to look closer.</h1>
        <p className="mt-[16px] text-[18px] leading-[1.65] text-[#596069]">Point to one part of your work that feels repetitive, slow, or more manual than it should be. Dystil will watch for that pattern first.</p>
      </header>

      <form className="mt-[38px] max-w-[860px]" onSubmit={(event) => { event.preventDefault(); submitRequest(); }}>
        <label htmlFor="fix-request" className="block text-[15px] font-medium text-[#252826]">What should Dystil pay attention to?</label>
        <div className="mt-[12px] overflow-hidden rounded-[12px] border border-[#d8d7d2] bg-white transition-colors focus-within:border-[#6ba893] focus-within:ring-1 focus-within:ring-[#6ba893]">
          <div className="px-[18px] py-[15px]">
            <textarea
              id="fix-request"
              value={request}
              rows={3}
              maxLength={600}
              onChange={(event) => { setRequest(event.target.value); setSent(false); }}
              onKeyDown={(event) => {
                if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
                  event.preventDefault();
                  submitRequest();
                }
              }}
              placeholder="Describe the task, where it happens, and what makes it annoying…"
              className="block min-h-[92px] max-h-[180px] w-full resize-y bg-transparent text-[18px] leading-[1.55] text-black outline-none placeholder:text-[#8b9095]"
            />
          </div>
          <div className="flex min-h-[62px] items-center justify-between gap-5 border-t border-[#eceae5] bg-[#fbfbfa] px-[15px] py-[9px]">
            <div className="flex min-w-0 items-center gap-[9px] text-[14px] text-[#737980]">
              <span className="h-[7px] w-[7px] shrink-0 rounded-full bg-[#12a77a]" aria-hidden="true" />
              <span>Stays on this computer</span>
              <span className="text-[#a1a5aa]">·</span>
              <span>{request.length}/600</span>
            </div>
            <button type="submit" disabled={!request.trim()} className="h-[42px] shrink-0 rounded-[9px] bg-[#087b5f] px-[20px] text-[16px] font-medium text-white transition-colors hover:bg-[#06634d] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#087b5f] disabled:cursor-not-allowed disabled:bg-[#d8dfdb] disabled:text-[#8a928e]">Look into this</button>
          </div>
        </div>
        <p className={`mt-[14px] min-h-[26px] text-[15px] ${sent ? "text-[#087b5f]" : "text-[#83888e]"}`} aria-live="polite">{sent ? "Got it. Dystil will look for this pattern first." : "Press ⌘ Enter to submit"}</p>
      </form>

      <section className="mt-[28px] max-w-[720px]">
        <p className="text-[14px] font-medium text-[#737980]">Not sure how to phrase it?</p>
        <div className="mt-[9px] divide-y divide-[#e7e5df] border-y border-[#e7e5df]">
          {suggestions.map((suggestion) => (
            <button key={suggestion} type="button" onClick={() => { setRequest(suggestion); setSent(false); }} className="group flex min-h-[48px] w-full items-center justify-between gap-5 text-left text-[16px] text-[#4f555c] transition-colors hover:text-[#087b5f] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#087b5f]">
              <span>{suggestion}</span><span className="text-[#a5aaa7] transition-transform group-hover:translate-x-1 group-hover:text-[#087b5f]" aria-hidden="true">→</span>
            </button>
          ))}
        </div>
      </section>
    </div>
  );
}

function Privacy({ onOpenSettings }: { onOpenSettings: () => void }) {
  const [notice, setNotice] = useState("");
  const showNotice = (message: string) => {
    setNotice(message);
    window.setTimeout(() => setNotice(""), 2600);
  };
  return (
    <div className="mx-auto max-w-[1124px] pb-10">
      <h1 className="text-[29px] font-medium tracking-[-0.02em] text-[#1a1c20]">What stays on this computer</h1>
      <p className="mt-[10px] max-w-[680px] text-[18px] leading-[1.65] text-[#60636b]">Everything Dystil has read is in one folder on this machine. Nothing has been sent anywhere, and there is no copy of it to ask for.</p>
      <div className="mt-[18px] rounded-[12px] bg-[#f3f8f5] px-[18px] py-[14px] text-[17px] leading-[1.55] text-[#60636b]">This page is the whole picture in one place. Anything you want to change is one click away in <button type="button" className="text-[#0f6e56]" onClick={onOpenSettings}>Settings</button>.</div>

      <PrivacyCard accent title="What never leaves">
        Everything Dystil reads about your work, and everything it has worked out from it.<br />Where it lives: <span className="text-[#1a1c20]">~/Library/Application Support/Dystil</span>
        <div className="mt-[14px] flex gap-5"><TextAction onClick={() => showNotice("Opening Dystil’s local folder…")}>Open that folder</TextAction><TextAction onClick={() => showNotice("Opening the source code…")}>Read the code that does this</TextAction></div>
      </PrivacyCard>

      <PrivacyCard title="What does leave, and only because you chose it" action={<TextAction onClick={onOpenSettings}>Change in Settings</TextAction>}>
        You are using your own Anthropic key, so anything Dystil needs a model for goes to Anthropic and is billed to you. It never comes to us.
      </PrivacyCard>

      <PrivacyCard title="Never read, and there is no switch for it">
        No job needs these, and getting them wrong would matter too much. They are skipped in the code, not in a setting.
        <ChipRow items={["Passwords and credentials", "Banking and payments", "Health", "Dating and faith apps", "Private browsing windows"]} />
      </PrivacyCard>

      <PrivacyCard title="Off unless you say otherwise" action={<TextAction onClick={onOpenSettings}>Change in Settings</TextAction>}>
        Personal for most people, work for some. You have turned one of these on.
        <ChipRow items={["Job boards and CVs, on", "Personal messaging", "Personal email", "HR and legal portals", "Payroll and salary"]} firstActive />
      </PrivacyCard>

      <PrivacyCard title="Delete what it has read">
        Deleting removes it from this computer, because that is the only place it was.
        <div className="mt-[14px] flex flex-wrap gap-[10px]">
          {["Everything from today", "One app or site", "A date range", "Everything, and start over"].map((item, index) => <button key={item} type="button" onClick={() => showNotice(index === 3 ? "Everything Dystil has read would be deleted. This cannot be undone." : `${item} deleted from this computer.`)} className={`rounded-[9px] border px-[15px] py-[9px] text-[15px] ${index === 3 ? "border-[#d9b4ae] text-[#9a4a3c]" : "border-[#e1e1dc] text-[#30333a]"}`}>{item}</button>)}
        </div>
      </PrivacyCard>

      <section className="mt-[28px] border-t border-[#e7e7e2] pt-[24px]">
        <h2 className="text-[22px] font-medium text-[#1a1c20]">How your work is read</h2>
        <p className="mt-[7px] max-w-[690px] text-[17px] leading-[1.6] text-[#60636b]">This is what Dystil thinks it is looking at. If any of it is wrong, or is none of its business, take it out.</p>
        <p className="mt-[5px] max-w-[690px] text-[16px] leading-[1.6] text-[#92969e]">Removing something here deletes it and stops it being used again. Turning a source off, below, stops Dystil reading it at all.</p>
        <DataChips label="The work you do" items={["Client reporting", "Month-end close", "Invoicing", "Hiring"]} />
        <DataChips label="People your work passes through" items={["Priya", "Rahul", "The Halloran team"]} />
        <div className="mt-[22px] rounded-[12px] border border-[#e7e7e2] bg-white px-[20px] py-[18px]">
          <div className="flex justify-between gap-5"><div><h3 className="text-[18px] font-medium">Apps and sites it has come across</h3><p className="mt-1 text-[16px] text-[#60636b]">Forty-one in total. These six are where nearly all of your work happens.</p></div><TextAction onClick={onOpenSettings}>Manage in Settings</TextAction></div>
          <ChipRow items={["Excel", "Outlook", "docs.google.com", "Slack", "Xero", "app.hubspot.com", "Instagram", "open.spotify.com"]} />
        </div>
      </section>
      {notice && <div role="status" className="fixed bottom-[28px] left-1/2 z-20 -translate-x-1/2 rounded-[9px] bg-[#1a1c20] px-[18px] py-[11px] text-[15px] text-white">{notice}</div>}
    </div>
  );
}

function PrivacyCard({ title, action, accent = false, children }: { title: string; action?: React.ReactNode; accent?: boolean; children: React.ReactNode }) {
  return <section className={`mt-[13px] rounded-[12px] border bg-white px-[21px] py-[18px] ${accent ? "border-[#c9e7db]" : "border-[#e7e7e2]"}`}><div className="flex items-start justify-between gap-6"><div className="min-w-0 flex-1"><h2 className={`text-[18px] font-medium ${accent ? "text-[#0f6e56]" : "text-[#1a1c20]"}`}>{title}</h2><div className="mt-[6px] text-[16px] leading-[1.6] text-[#60636b]">{children}</div></div>{action}</div></section>;
}

function TextAction({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return <button type="button" onClick={onClick} className="shrink-0 text-[15px] text-[#0f6e56] hover:text-[#094b3b] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#1d9e75]">{children}</button>;
}

function ChipRow({ items, firstActive = false }: { items: string[]; firstActive?: boolean }) {
  return <div className="mt-[14px] flex flex-wrap gap-[8px]">{items.map((item, index) => <span key={item} className={`rounded-full border px-[13px] py-[6px] text-[14px] ${firstActive && index === 0 ? "border-[#c9e7db] bg-[#f3f8f5] text-[#0f6e56]" : "border-[#e1e1dc] text-[#60636b]"}`}>{item}</span>)}</div>;
}

function DataChips({ label, items }: { label: string; items: string[] }) {
  const [visible, setVisible] = useState(items);
  return <div className="mt-[20px]"><p className="mb-[9px] text-[14px] text-[#92969e]">{label}</p><div className="flex flex-wrap gap-[8px]">{visible.map((item) => <button key={item} type="button" onClick={() => setVisible((current) => current.filter((value) => value !== item))} className="rounded-full bg-[#f3f8f5] px-[13px] py-[6px] text-[14px] text-[#26312d]">{item} <span className="ml-1 text-[#9a9da5]">×</span></button>)}</div></div>;
}

const settingsTabs: readonly SettingsTab[] = ["What Dystil can see", "Model and cost", "When it runs", "Storage", "Notifications", "Invite your team", "About"];

function SettingsWorkspace(props: Props & { initialTab: SettingsTab; onBack: () => void }) {
  const [tab, setTab] = useState<SettingsTab>(props.initialTab);
  return <div className="grid h-full grid-cols-[268px_minmax(0,1fr)]"><aside className="border-r border-[#e3e3de] bg-white py-[27px]"><button type="button" onClick={props.onBack} className="px-[27px] text-[17px] text-[#60636b] hover:text-[#0f6e56]">←&nbsp; Back to Dystil</button><p className="mb-[10px] mt-[28px] px-[27px] text-[13px] tracking-[0.08em] text-[#9a9da5]">SETTINGS</p><nav>{settingsTabs.map((item) => <button key={item} type="button" onClick={() => setTab(item)} className={`relative block min-h-[47px] w-full px-[27px] text-left text-[17px] ${tab === item ? "bg-[#e1f5ee] text-[#0f6e56]" : "text-[#60636b] hover:bg-[#f8f8f7]"}`}>{tab === item && <span className="absolute inset-y-0 left-0 w-[2px] bg-[#1d9e75]" />}{item}</button>)}</nav></aside><section className="min-h-0 overflow-y-auto px-[48px] py-[42px]"><SettingsPane tab={tab} {...props} /></section></div>;
}

function SettingsPane({ tab, recording, toggling, onToggleRecording, screenshotEnabled, onScreenshotChange, screenshotBusy, userName, userEmail, onLogout, loggingOut, version }: Props & { tab: SettingsTab }) {
  if (tab === "What Dystil can see") return <SettingsPage title="What Dystil can see" lede="Changes here apply from now on. To delete something it has already read, go to What stays on this computer."><SettingLabel>Off unless you turn them on</SettingLabel><SettingsCard>{[["Personal messaging", "WhatsApp, iMessage, Telegram", false], ["Personal email", "", false], ["Job boards and CVs", "Turn on if you hire or recruit", true], ["HR and legal portals", "", false], ["Payroll and salary", "", false]].map(([title, description, enabled]) => <ToggleRow key={String(title)} title={String(title)} description={String(description)} initial={Boolean(enabled)} />)}</SettingsCard><div className="mb-[10px] mt-[28px] flex justify-between"><SettingLabel>Apps and sites it has come across</SettingLabel><span className="text-[14px] text-[#9a9da5]">2 turned off</span></div><SettingsCard><div className="border-b border-[#efefe9] px-[17px] py-[12px] text-[15px] text-[#9a9da5]">Find an app or site</div><div className="flex items-center justify-between bg-[#f8f8f7] px-[18px] py-[13px]"><div><p className="text-[16px] font-medium">Where most of your work happens</p><p className="mt-1 text-[14px] text-[#9a9da5]">Worth a look. These shape almost every finding.</p></div><span className="text-[14px] text-[#9a9da5]">6</span></div>{["Excel", "Outlook", "docs.google.com", "Slack", "Xero", "app.hubspot.com"].map((item) => <ToggleRow key={item} title={item} description="Used this month" initial />)}</SettingsCard><p className="mt-[14px] text-[14px] leading-[1.6] text-[#9a9da5]">Anything new is read by default. The categories Dystil never reads at all are listed on What stays on this computer.</p></SettingsPage>;
  if (tab === "Model and cost") return <SettingsPage title="Model and cost" lede="Where the model runs, and what it has cost you."><SettingsCard padded><div className="flex justify-between gap-6"><div><h2 className="text-[18px] font-medium">Your own Anthropic key</h2><p className="mt-1 text-[16px] leading-[1.55] text-[#60636b]">Findings are generated by Anthropic and billed to you. Nothing reaches Dystil.</p></div><TextAction onClick={() => undefined}>Change</TextAction></div><div className="mt-[16px] rounded-[9px] bg-[#f8f8f7] px-[14px] py-[11px] text-[15px] text-[#60636b]">sk-ant-••••••••••••7f2a</div></SettingsCard><SettingsCard padded><h2 className="text-[18px] font-medium">What it has cost you</h2><div className="mt-[16px] flex gap-[52px]"><Stat value="$1.84" label="This month" /><Stat value="$0.06" label="Per finding, average" /><Stat value="31" label="Findings generated" /></div><p className="mt-[18px] text-[14px] text-[#9a9da5]">Estimated from your provider’s published prices. Your actual bill is the one they send you.</p></SettingsCard><SettingsCard padded><div className="flex justify-between"><div><h2 className="text-[18px] font-medium">Cap what it spends</h2><p className="mt-1 text-[16px] text-[#60636b]">Dystil stops generating findings past this and tells you.</p></div><span className="text-[22px]">$10</span></div><input className="mt-[20px] w-full accent-[#1d9e75]" type="range" min="5" max="50" defaultValue="10" /></SettingsCard></SettingsPage>;
  if (tab === "When it runs") return <SettingsPage title="When it runs" lede="Dystil works in the background whether or not this window is open. If it is not running while you work, it has nothing to tell you later."><SettingsCard><ToggleRow title="Start when you log in" description="No window opens. It sits in the menu bar." initial /><ToggleRow title="Capture screenshots" description="Include visual context in local capture." initial={screenshotEnabled} onChange={onScreenshotChange} disabled={screenshotBusy} /></SettingsCard><SettingsCard padded><div className="flex items-center justify-between gap-5"><div><h2 className="text-[18px] font-medium">Pause</h2><p className="mt-1 text-[16px] text-[#60636b]">Also in the menu bar, any time, without opening this.</p></div><div className="flex gap-2"><SmallButton onClick={onToggleRecording} disabled={toggling}>{toggling ? "Updating…" : recording ? "1 hour" : "Resume"}</SmallButton><SmallButton onClick={onToggleRecording} disabled={toggling}>Today</SmallButton></div></div></SettingsCard></SettingsPage>;
  if (tab === "Storage") return <SettingsPage title="Storage" lede="How much of this machine Dystil is allowed to use, and how far back it can therefore remember."><SettingsCard padded><p className="text-[25px] font-medium">1.2 GB used of 4 GB</p><p className="mt-1 text-[14px] text-[#9a9da5]">Growing about 300 MB a month</p><SettingLabel className="mt-[22px]">How much Dystil may use</SettingLabel><input className="mt-[9px] w-full accent-[#1d9e75]" type="range" min="1" max="12" defaultValue="4" /></SettingsCard><div className="rounded-[12px] bg-[#f3f8f5] px-[20px] py-[18px]"><h2 className="text-[18px] font-medium text-[#0f6e56]">About a year of your work</h2><p className="mt-1 text-[16px] leading-[1.6] text-[#60636b]">Enough to see patterns that only show up across quarters. When it fills, Dystil drops the oldest weeks first.</p></div></SettingsPage>;
  if (tab === "Notifications") return <SettingsPage title="Notifications" lede="Dystil interrupts rarely and only when it has something. There are no reminders, streaks, or nudges to come back."><SettingsCard><ToggleRow title="When it finds something worth telling you" description="Roughly once a week, often less" initial /><ToggleRow title="When something you asked for is ready" description="" initial /></SettingsCard></SettingsPage>;
  if (tab === "Invite your team") return <SettingsPage title="Invite your team" lede="Dystil can only see your machine. The work your team repeats across each other is invisible from here, and that is usually where the real time goes."><SettingsCard padded accent><div className="flex gap-[52px]"><Stat value="6 hrs" label="Found on your machine" /><Stat value="?" label="Across everyone else" accent /></div><p className="mt-[22px] text-[16px] leading-[1.6] text-[#60636b]">Send this to someone and they get their own findings, on their own machine. Nothing of yours is shared with them.</p><button type="button" className="mt-[18px] rounded-[9px] bg-[#0f6e56] px-[18px] py-[11px] text-[16px] font-medium text-white">Copy an invite link</button></SettingsCard></SettingsPage>;
  return <SettingsPage title="About" lede=""><SettingsCard><SimpleRow title={`Version ${version || "1.0.4"}`} action="Check now" /><ToggleRow title="Update automatically" description="Installs quietly. Off means you get a banner instead." initial /><SimpleRow title="Read the source code" action="Open" /></SettingsCard><SettingsCard padded><div className="flex justify-between"><span className="text-[16px]">{userEmail || userName}</span><button type="button" onClick={onLogout} disabled={loggingOut} className="text-[15px] text-[#60636b]">{loggingOut ? "Signing out…" : "Sign out"}</button></div></SettingsCard></SettingsPage>;
}

function SettingsPage({ title, lede, children }: { title: string; lede: string; children: React.ReactNode }) { return <div className="mx-auto max-w-[880px]"><h1 className="text-[29px] font-medium tracking-[-0.02em]">{title}</h1>{lede && <p className="mb-[26px] mt-[8px] max-w-[690px] text-[18px] leading-[1.6] text-[#60636b]">{lede}</p>}{children}</div>; }
function SettingsCard({ children, padded = false, accent = false }: { children: React.ReactNode; padded?: boolean; accent?: boolean }) { return <div className={`mb-[14px] overflow-hidden rounded-[12px] border bg-white ${accent ? "border-[#c9e7db]" : "border-[#e7e7e2]"} ${padded ? "p-[20px]" : ""}`}>{children}</div>; }
function SettingLabel({ children, className = "" }: { children: React.ReactNode; className?: string }) { return <p className={`mb-[10px] text-[14px] text-[#9a9da5] ${className}`}>{children}</p>; }
function ToggleRow({ title, description, initial, onChange, disabled = false }: { title: string; description: string; initial: boolean; onChange?: (value: boolean) => void; disabled?: boolean }) { const [on, setOn] = useState(initial); return <div className="flex min-h-[66px] items-center justify-between gap-5 border-b border-[#efefe9] px-[19px] py-[13px] last:border-b-0"><div><p className="text-[17px]">{title}</p>{description && <p className="mt-1 text-[14px] text-[#9a9da5]">{description}</p>}</div><button type="button" disabled={disabled} aria-pressed={on} onClick={() => { const value = !on; setOn(value); onChange?.(value); }} className={`h-[26px] w-[46px] rounded-full p-[3px] ${on ? "bg-[#1d9e75]" : "bg-[#e7e7e2]"}`}><span className={`block h-[20px] w-[20px] rounded-full bg-white transition-transform ${on ? "translate-x-[20px]" : ""}`} /></button></div>; }
function Stat({ value, label, accent = false }: { value: string; label: string; accent?: boolean }) { return <div><p className={`text-[28px] font-medium ${accent ? "text-[#0f6e56]" : ""}`}>{value}</p><p className="mt-1 text-[14px] text-[#9a9da5]">{label}</p></div>; }
function SmallButton({ onClick, disabled, children }: { onClick: () => void; disabled?: boolean; children: React.ReactNode }) { return <button type="button" disabled={disabled} onClick={onClick} className="rounded-[9px] border border-[#e1e1dc] bg-white px-[14px] py-[9px] text-[15px] disabled:opacity-50">{children}</button>; }
function SimpleRow({ title, action }: { title: string; action: string }) { return <div className="flex min-h-[62px] items-center justify-between border-b border-[#efefe9] px-[19px] py-[13px] last:border-b-0"><span className="text-[17px]">{title}</span><button type="button" className="text-[15px] text-[#0f6e56]">{action}</button></div>; }

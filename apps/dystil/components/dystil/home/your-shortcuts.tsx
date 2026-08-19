"use client";

import { useState } from "react";
import { Check, ChevronRight, Clipboard, ExternalLink, Sparkles } from "lucide-react";
import { useRouter } from "next/navigation";

import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useHome } from "@/lib/home/provider";
import type { Shortcut } from "@/lib/home/types";
import { commands } from "@/lib/utils/tauri";

const WORDS = ["No", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine"];
const BUILD_STEPS = [
  ["preparing", "Preparing"],
  ["investigating", "Understanding the work"],
  ["building", "Writing the skill"],
  ["validating", "Checking the bundle"],
] as const;

type InstallSurface = "claude" | "chatgpt";
type BundleStage = NonNullable<Shortcut["bundle"]>["stage"];

function countWord(n: number) {
  return WORDS[n] ?? String(n);
}

function BuildProgress({ stage }: { stage?: BundleStage }) {
  const active = Math.max(0, BUILD_STEPS.findIndex(([id]) => id === stage));
  return (
    <div className="mt-4 max-w-[520px]" aria-live="polite">
      <div className="mb-2 flex items-center justify-between text-ui-sm text-muted-ink">
        <span className="font-medium text-ink">Building your reusable skill</span>
        <span>{BUILD_STEPS[active][1]}…</span>
      </div>
      <ol className="flex items-center gap-1" aria-label="Skill build progress">
        {BUILD_STEPS.map(([id, label], index) => {
          const completed = index < active;
          const current = index === active;
          return (
            <li key={id} className="flex min-w-0 flex-1 items-center gap-1.5">
              <span className={`flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-[10px] font-bold ${completed ? "bg-sage text-paper" : current ? "bg-ink text-paper" : "bg-line-3 text-muted-ink"}`}>
                {completed ? <Check className="h-3 w-3" aria-hidden /> : index + 1}
              </span>
              <span className={`hidden truncate text-[11px] sm:inline ${current ? "font-semibold text-ink" : "text-muted-ink"}`}>{label}</span>
            </li>
          );
        })}
      </ol>
      <p className="mt-2 text-ui-sm text-muted-ink">This can take a few minutes. You can leave this screen; Dystil will keep working.</p>
    </div>
  );
}

function GuidedScreenshot({
  src,
  alt,
  callouts,
}: {
  src: string;
  alt: string;
  callouts: Array<{ label: string; className: string; arrow?: "↓" | "↑" }>;
}) {
  return (
    <div className="relative mt-2 overflow-hidden rounded-[10px] border border-line-2 bg-ink">
      <img src={src} alt={alt} className="block w-full" />
      {callouts.map(({ label, className, arrow = "↓" }) => <span key={label} aria-hidden className={`pointer-events-none absolute inline-flex items-center gap-1 rounded-[5px] bg-[#f3a34b] px-2 py-1 text-[10px] font-bold text-ink shadow-sm ${className}`}>{label}<span className="text-sm leading-none">{arrow}</span></span>)}
    </div>
  );
}

function InstallDialog({ shortcut, onClose }: { shortcut: Shortcut | null; onClose: () => void }) {
  const { copyShortcut, installShortcutSkill, exportShortcutSkill } = useHome();
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState<"codex" | "claude" | "chatgpt" | "prompt" | null>(null);
  const [surface, setSurface] = useState<InstallSurface | null>(null);
  const targets = shortcut?.bundle?.targets ?? [];
  const hasLocalTarget = (target: "codex" | "claude") => targets.some((item) => item.target === target && item.available);
  const skillName = shortcut?.bundle?.skillName ?? "your-skill-name";
  const invocation = surface === "claude" ? `/${skillName}` : `@${skillName}`;

  async function copyPrompt() {
    if (!shortcut) return;
    setBusy("prompt");
    const copied = await copyShortcut(shortcut.id);
    setBusy(null);
    setNotice(copied
      ? "Prompt copied. Paste it into any chat."
      : "Dystil could not copy the prompt. Try again, or use a desktop app instead.");
  }

  async function installLocal(target: "codex" | "claude") {
    if (!shortcut) return;
    setBusy(target);
    const installed = await installShortcutSkill(shortcut.id, target);
    setBusy(null);
    setNotice(installed
      ? `Installed for ${target === "codex" ? "Codex" : "Claude Code"}.`
      : `Dystil could not install the skill in ${target === "codex" ? "Codex" : "Claude Code"}. Check that it is installed, then try again.`);
  }

  async function exportAndOpen(surface: InstallSurface) {
    if (!shortcut) return;
    setBusy(surface);
    const receipt = await exportShortcutSkill(shortcut.id);
    if (!receipt) {
      setNotice("Dystil could not prepare the ZIP. Try again in a moment.");
    } else {
      const [reveal, launch] = await Promise.allSettled([
        commands.revealSkillBundleExport(receipt.bundleId),
        commands.openSkillBundleProvider(surface),
      ]);
      if (reveal.status === "rejected" || launch.status === "rejected" || launch.value.status === "error") {
        setNotice("The ZIP was prepared, but Dystil could not open part of the handoff. You can still use the selected export from Files.");
      } else if (surface === "claude") {
        setNotice(launch.value.data.destination === "desktop"
          ? "The ZIP is selected in your files and Claude Desktop is open. Attach it where your Claude surface accepts skill or project files."
          : "The ZIP is selected in your files and Claude is open in your browser. Attach it where your Claude surface accepts skill or project files.");
      } else {
        setNotice(launch.value.data.destination === "desktop"
          ? "The ZIP is selected in your files and ChatGPT Desktop is open. Go to Plugins → Skills → Create → Upload from your computer."
          : "The ZIP is selected in your files and ChatGPT is open in your browser. Go to Plugins → Skills → Create → Upload from your computer.");
      }
    }
    setBusy(null);
  }

  function close() {
    setSurface(null);
    setNotice(null);
    onClose();
  }

  return (
    <Dialog open={Boolean(shortcut)} onOpenChange={(open) => !open && close()}>
      <DialogContent className="max-h-[90vh] max-w-[590px] overflow-y-auto rounded-[16px] border-line-2 bg-paper p-0 shadow-card" overlayClassName="bg-ink/30">
        <DialogHeader className="border-b border-line-2 px-7 pb-4 pt-6 text-left">
          <DialogTitle className="pr-8 font-display text-[25px] font-normal leading-tight text-ink">Use this skill</DialogTitle>
          <DialogDescription className="sr-only">Choose an AI app for this skill, then follow its import steps.</DialogDescription>
        </DialogHeader>

        <div className="space-y-3 px-7 py-5">
          {!surface ? <>
            <p className="pb-1 text-ui-sm text-muted-ink">Choose where you want to add it. We’ll show the exact import steps before opening anything.</p>
            <button type="button" onClick={() => setSurface("claude")} className="group flex w-full items-center gap-4 rounded-[12px] border border-line-2 px-4 py-4 text-left transition-colors hover:border-sage-border hover:bg-sage-pale"><span className="flex h-9 w-9 items-center justify-center rounded-[9px] bg-[#fff1eb]"><img src="/brand/claude.svg" alt="" className="h-5 w-5" /></span><span className="min-w-0 flex-1"><span className="block font-semibold text-ink">Claude Desktop</span><span className="block text-ui-sm text-muted-ink">Upload the portable skill in Claude’s Skills settings.</span></span><ChevronRight className="h-4 w-4 text-muted-ink" /></button>
            <button type="button" onClick={() => setSurface("chatgpt")} className="group flex w-full items-center gap-4 rounded-[12px] border border-line-2 px-4 py-4 text-left transition-colors hover:border-sage-border hover:bg-sage-pale"><span className="flex h-9 w-9 items-center justify-center rounded-[9px] bg-[#f1f1ef]"><img src="/brand/chatgpt.svg" alt="" className="h-5 w-5" /></span><span className="min-w-0 flex-1"><span className="block font-semibold text-ink">ChatGPT</span><span className="block text-ui-sm text-muted-ink">Upload the portable skill from ChatGPT Skills.</span></span><ChevronRight className="h-4 w-4 text-muted-ink" /></button>
          </> : <>
            <button type="button" onClick={() => setSurface(null)} className="mb-2 text-ui-sm font-medium text-ink-3 hover:text-ink">← Choose another app</button>
            <h3 className="font-semibold text-ink">Install in {surface === "claude" ? "Claude Desktop" : "ChatGPT"}</h3>
            <div className="rounded-[10px] bg-sage-pale px-4 py-3 text-ui-sm leading-5 text-ink">
              <span className="block font-semibold">Use it after upload</span>
              In a new {surface === "claude" ? "Claude" : "ChatGPT"} chat, type <code className="mx-1 rounded bg-paper px-1.5 py-0.5 font-mono text-[12px] font-semibold">{invocation}</code> then describe the work you want done.
            </div>
            {surface === "claude" ? <ol className="space-y-5 text-ui-sm text-ink"><li><span className="font-semibold">1. Open Customize</span><GuidedScreenshot src="/skill-install/claude-customize.png" alt="Claude Desktop home screen with an arrow pointing to Customize in the sidebar" callouts={[{ label: "Click Customize", className: "left-[1%] top-[14%]" }]} /></li><li><span className="font-semibold">2. Select Skills</span><GuidedScreenshot src="/skill-install/claude-skills.png" alt="Claude Desktop Settings with an arrow pointing to Skills in the sidebar" callouts={[{ label: "Click Skills", className: "left-[6%] top-[72%]" }]} /></li><li><span className="font-semibold">3. Add → Upload a skill</span><GuidedScreenshot src="/skill-install/claude-upload.png" alt="Claude Skills page with arrows pointing to Add and Upload a skill" callouts={[{ label: "1. Click Add", className: "left-[75%] top-[1%]" }, { label: "2. Then Upload a skill", className: "left-[67%] top-[12%]" }]} /></li><li className="text-muted-ink">Choose the ZIP Dystil will reveal in your file manager.</li></ol> : <ol className="space-y-4 text-ui-sm text-ink"><li><span className="font-semibold">Open Plugins → Skills → + → Upload from your computer</span><GuidedScreenshot src="/skill-install/chatgpt-upload.png" alt="ChatGPT Skills page with arrows pointing to Plugins, Skills, the plus button, and Upload from your computer" callouts={[{ label: "1. Plugins", className: "left-[3%] top-[20%]" }, { label: "2. Skills", className: "left-[52%] top-[5%]", arrow: "↑" }, { label: "3. Click +", className: "left-[74%] top-[7%]" }, { label: "4. Upload from your computer", className: "left-[57%] top-[25%]" }]} /></li><li className="text-muted-ink">Choose the ZIP Dystil will reveal in your file manager.</li></ol>}
            <button type="button" disabled={busy !== null} onClick={() => void exportAndOpen(surface)} className="mt-2 flex w-full items-center justify-center gap-2 rounded-icon bg-ink px-4 py-3 text-ui-sm font-semibold text-paper hover:bg-ink-2 disabled:opacity-45"><ExternalLink className="h-4 w-4" />{busy === surface ? "Preparing your ZIP…" : `Open ${surface === "claude" ? "Claude" : "ChatGPT"} and reveal ZIP`}</button>
          </>}

          <details className="mt-5 border-t border-line-2 pt-4"><summary className="cursor-pointer text-ui-sm font-medium text-ink-3 hover:text-ink">CLI & more options</summary><div className="mt-3 space-y-3">
          <button type="button" disabled={!hasLocalTarget("codex") || busy !== null} onClick={() => void installLocal("codex")} className="group flex w-full items-center gap-4 rounded-[12px] border border-line-2 px-4 py-4 text-left transition-colors hover:border-sage-border hover:bg-sage-pale disabled:cursor-not-allowed disabled:opacity-45">
            <span className="flex h-9 w-9 items-center justify-center rounded-[9px] bg-[#f1f1ef]"><img src="/brand/chatgpt.svg" alt="" className="h-5 w-5" /></span>
            <span className="min-w-0 flex-1"><span className="block font-semibold text-ink">{busy === "codex" ? "Installing in Codex…" : "Install in Codex"}</span><span className="block text-ui-sm text-muted-ink">{hasLocalTarget("codex") ? "Adds it to the Codex skills folder on this computer." : "Codex is not detected on this computer."}</span></span>
            <ChevronRight className="h-4 w-4 text-muted-ink transition-transform group-hover:translate-x-0.5" />
          </button>

          <button type="button" disabled={!hasLocalTarget("claude") || busy !== null} onClick={() => void installLocal("claude")} className="group flex w-full items-center gap-4 rounded-[12px] border border-line-2 px-4 py-4 text-left transition-colors hover:border-sage-border hover:bg-sage-pale disabled:cursor-not-allowed disabled:opacity-45">
            <span className="flex h-9 w-9 items-center justify-center rounded-[9px] bg-[#fff1eb]"><img src="/brand/claude.svg" alt="" className="h-5 w-5" /></span>
            <span className="min-w-0 flex-1"><span className="block font-semibold text-ink">{busy === "claude" ? "Installing in Claude Code…" : "Install in Claude Code"}</span><span className="block text-ui-sm text-muted-ink">{hasLocalTarget("claude") ? "Adds it to the Claude Code skills folder on this computer." : "Claude Code is not detected on this computer."}</span></span>
            <ChevronRight className="h-4 w-4 text-muted-ink transition-transform group-hover:translate-x-0.5" />
          </button>
          <button type="button" disabled={busy !== null} onClick={() => void copyPrompt()} className="flex w-full items-center gap-4 px-4 py-3 text-left text-ui-sm font-medium text-ink-3 transition-colors hover:text-ink disabled:opacity-45"><Clipboard className="h-4 w-4" />{busy === "prompt" ? "Copying prompt…" : "Just copy the prompt"}</button>
          </div></details>
          {notice ? <p className="rounded-[8px] bg-sage-pale px-3 py-2 text-ui-sm leading-5 text-ink" role="status">{notice}</p> : null}
        </div>
      </DialogContent>
    </Dialog>
  );
}

export function YourShortcuts() {
  const router = useRouter();
  const { shortcuts, buildShortcutSkill } = useHome();
  const [installing, setInstalling] = useState<Shortcut | null>(null);

  function openInstall(shortcut: Shortcut) {
    setInstalling(shortcut);
    void commands.recordSkillBundleInstallIntent().catch(() => {});
  }

  return (
    <div className="mx-auto max-w-[760px] px-10 pb-[50px] pt-8">
      <div className="mb-[26px] flex items-center gap-[11px]"><span className="text-meta text-ink-2"><span className="font-semibold">{shortcuts.length} kept</span> · ready when you are</span><div className="flex-1" /><button type="button" onClick={() => router.push("/home/ask")} className="-mr-[10px] whitespace-nowrap rounded-strip px-[10px] py-[5px] text-meta font-semibold text-ink-3 transition-colors hover:bg-chrome hover:text-ink">Ask for another</button></div>
      <h1 className="mb-[26px] max-w-[30ch] text-pretty font-display text-display font-normal text-ink">{shortcuts.length === 0 ? "Nothing kept yet." : `${countWord(shortcuts.length)} ${shortcuts.length === 1 ? "thing" : "things"} you no longer do by hand.`}</h1>
      <div className="flex flex-col gap-[9px]">
        {shortcuts.map((shortcut) => {
          const building = shortcut.bundle?.status === "pending" || shortcut.bundle?.status === "running";
          const interrupted = shortcut.bundle?.status === "interrupted";
          const retryable = shortcut.bundle?.status === "failed" || interrupted;
          return <div key={shortcut.id} className="rounded-[12px] border border-line-2 bg-paper px-[18px] py-[15px] transition-shadow hover:border-sage-border hover:shadow-card-hover">
            <div className="flex items-center gap-[18px]"><div className="min-w-0 flex-1"><div className="mb-[3px] flex items-center gap-[9px]"><span className="truncate text-[15px] font-semibold text-ink">{shortcut.title}</span><span className="shrink-0 rounded-[4px] bg-line-3 px-[7px] py-[2px] text-[10px] font-bold uppercase tracking-[0.09em] text-muted-ink">{shortcut.kind}</span></div><div className="text-ui-sm text-muted-ink">{shortcut.meta}</div></div>{shortcut.bundle?.status === "ready" ? <button type="button" onClick={() => openInstall(shortcut)} className="shrink-0 rounded-icon bg-ink px-[14px] py-2 text-ui-sm font-semibold text-paper transition-colors hover:bg-ink-2">Install</button> : <button type="button" disabled={building} onClick={() => void buildShortcutSkill(shortcut.id)} className="shrink-0 rounded-icon px-[13px] py-2 text-ui-sm font-medium text-ink-3 transition-colors hover:bg-chrome hover:text-ink disabled:opacity-50">{retryable ? "Retry build" : building ? "Building…" : "Build skill"}</button>}</div>
            {building ? <BuildProgress stage={shortcut.bundle?.stage} /> : null}
            {shortcut.bundle?.status === "failed" ? <div role="alert" className="mt-3 text-ui-sm text-red-600">Couldn’t build this skill. Try again.</div> : null}
            {interrupted ? <div role="alert" className="mt-3 text-ui-sm text-muted-ink">Dystil was closed before this skill finished. Retry build to start again.</div> : null}
          </div>;
        })}
      </div>
      <InstallDialog shortcut={installing} onClose={() => setInstalling(null)} />
    </div>
  );
}

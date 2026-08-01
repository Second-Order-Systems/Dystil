"use client";

import { type ReactNode, useEffect, useState } from "react";
import { Check, Download } from "lucide-react";

import { cn } from "@/lib/utils";

type Choice = "codex" | "claude" | "local";
type Setup = { choice: Choice };

export function OnboardingAiSetupStep({ onReadyChange, onSetupChange }: {
  onReadyChange: (ready: boolean) => void;
  onSetupChange: (setup: Setup | null) => void;
}) {
  const [choice, setChoice] = useState<Choice | null>(null);
  const ready = choice !== null;

  useEffect(() => {
    onReadyChange(ready);
    onSetupChange(choice ? { choice } : null);
  }, [choice, onReadyChange, onSetupChange, ready]);

  return <div className="space-y-4">
    <p className="max-w-[58ch] text-sm leading-6 text-muted-foreground">Ask Your Work needs an AI preset. Choose a subscription now, or finish setup and add Ollama or an OpenAI-compatible endpoint from Settings.</p>
    <fieldset className="space-y-3"><legend className="text-sm font-semibold">AI preset</legend><div role="radiogroup" aria-label="AI preset" className="grid gap-3 md:grid-cols-2"><ProviderChoice selected={choice === "codex"} title="ChatGPT subscription" description="Use the official Codex client with your ChatGPT subscription." onSelect={() => setChoice("codex")} /><ProviderChoice selected={choice === "claude"} title="Claude subscription" description="Use the official Claude Code client." onSelect={() => setChoice("claude")} /></div><p className="text-xs leading-5 text-muted-foreground">Dystil prepares the selected subscription client after Finish. You decide when to sign in.</p></fieldset>
    <div className="divide-y divide-border border-y border-border">
      <SetupRow selected={choice === "local"} title="Set up later" description="Dystil will keep capturing and indexing locally. Add ChatGPT, Ollama, or a custom provider from Settings when ready." onSelect={() => setChoice("local")} icon={<Download />} />
    </div>
  </div>;
}

function ProviderChoice({ selected, title, description, onSelect }: { selected: boolean; title: string; description: string; onSelect: () => void }) {
  return <button type="button" role="radio" aria-checked={selected} onClick={onSelect} className={cn("flex min-h-24 items-start justify-between rounded-lg border px-4 py-3 text-left transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring", selected ? "border-primary bg-primary/[.05]" : "border-border hover:border-primary/45 hover:bg-muted/40")}><span><span className="block text-sm font-semibold">{title}</span><span className="mt-1 block text-xs leading-5 text-muted-foreground">{description}</span></span><span aria-hidden className={cn("mt-0.5 grid h-4 w-4 shrink-0 place-items-center rounded-full border", selected ? "border-primary" : "border-muted-foreground/60")}><span className={cn("h-2 w-2 rounded-full bg-primary transition", selected ? "scale-100" : "scale-0")} /></span></button>;
}

function SetupRow({ selected, title, description, onSelect, action, icon }: { selected: boolean; title: string; description: string; onSelect: () => void; action?: ReactNode; icon?: ReactNode }) {
  return <div className={cn("grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 px-1 py-1 transition", selected ? "bg-primary/[.04]" : "hover:bg-muted/50")}><button type="button" onClick={onSelect} className="min-w-0 py-3 text-left"><span className="flex items-center gap-2 text-sm font-semibold">{icon && <span className="text-primary [&_svg]:h-4 [&_svg]:w-4">{icon}</span>}{title}{selected && <Check className="h-4 w-4 text-primary" />}</span><span className="mt-1 block max-w-[60ch] text-xs leading-5 text-muted-foreground">{description}</span></button>{action && <span>{action}</span>}</div>;
}

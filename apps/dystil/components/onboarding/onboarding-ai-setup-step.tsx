"use client";

import { type ReactNode, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, Download, KeyRound, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";

type Choice = "codex" | "claude" | "byok" | "local";
type Setup = { choice: Choice; enableLocalProcessing: boolean };

export function OnboardingAiSetupStep({ onReadyChange, onSetupChange }: {
  onReadyChange: (ready: boolean) => void;
  onSetupChange: (setup: Setup | null) => void;
}) {
  const [choice, setChoice] = useState<Choice | null>(null);
  const [busy, setBusy] = useState<Choice | null>(null);
  const [message, setMessage] = useState("");
  const [endpoint, setEndpoint] = useState("https://api.openai.com");
  const [apiKey, setApiKey] = useState("");
  const [chatModel, setChatModel] = useState("gpt-5.6-luna");
  const [processingModel, setProcessingModel] = useState("gpt-5.6-luna");
  const [byokReady, setByokReady] = useState(false);
  const ready = choice === "codex" || choice === "claude" || choice === "local" || (choice === "byok" && byokReady);
  useEffect(() => {
    onReadyChange(ready);
    onSetupChange(choice && ready ? { choice, enableLocalProcessing: false } : null);
  }, [choice, onReadyChange, onSetupChange, ready]);

  const chooseProvider = (provider: "codex" | "claude") => {
    if (busy) return;
    setChoice(provider);
    setMessage("");
  };

  const saveByok = async () => {
    if (!apiKey.trim() || busy) return;
    setBusy("byok"); setMessage("Saving your API key…");
    try {
      await invoke("byok_save_profile", { endpoint, chatModel, workCardModel: processingModel, apiKey });
      setApiKey(""); setByokReady(true);
      setMessage("Your API key is stored in this device’s credential store. Dystil is ready to answer questions and organize activity.");
    } catch (error) { setMessage(error instanceof Error ? error.message : "Could not save this API key."); }
    finally { setBusy(null); }
  };

  return <div className="space-y-4" aria-busy={busy !== null}>
    <p className="max-w-[58ch] text-sm leading-6 text-muted-foreground">Ask Your Work needs a connected provider to answer questions and create useful activity summaries. Choose a subscription, use your own API key, or set this up later.</p>
    <fieldset className="space-y-3"><legend className="text-sm font-semibold">Chat provider</legend><div role="radiogroup" aria-label="Chat provider" className="grid gap-3 md:grid-cols-3"><ProviderChoice selected={choice === "codex"} disabled={busy !== null} title="ChatGPT Plus" description="Use your ChatGPT subscription." onSelect={() => chooseProvider("codex")} /><ProviderChoice selected={choice === "claude"} disabled={busy !== null} title="Claude Pro" description="Use your Claude subscription." onSelect={() => chooseProvider("claude")} /><ProviderChoice selected={choice === "byok"} disabled={busy !== null} title="Your API key" description="Use an OpenAI-compatible provider." onSelect={() => { setChoice("byok"); setMessage(""); }} /></div><p className="text-xs leading-5 text-muted-foreground">Dystil sets up subscription connectors after Finish. You decide when to sign in.</p></fieldset>
    <div className="divide-y divide-border border-y border-border">
      <SetupRow selected={choice === "local"} disabled={busy !== null} title="Set up later" description="Dystil will keep capturing and indexing locally. Connect a provider from Settings when you are ready to ask questions." onSelect={() => setChoice("local")} icon={<Download />} />
    </div>
    {choice === "byok" && <section aria-labelledby="byok-title" className="border-y border-border py-5"><div className="flex items-start justify-between gap-5"><div><h2 id="byok-title" className="flex items-center gap-2 text-sm font-semibold"><KeyRound className="h-4 w-4 text-primary" />Connect your API key</h2><p className="mt-1 max-w-[58ch] text-xs leading-5 text-muted-foreground">Your key stays in this device’s credential store. Dystil uses it for Ask Your Work and activity organization.</p></div>{byokReady && <span className="flex items-center gap-1 text-xs font-medium text-primary"><Check className="h-4 w-4" />Saved</span>}</div><div className="mt-4 grid gap-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-end"><label className="grid gap-1.5 text-xs font-semibold">API key<Input type="password" value={apiKey} onChange={(event) => { setApiKey(event.target.value); setByokReady(false); }} autoComplete="off" placeholder="Paste your provider key" className="h-11 rounded-lg bg-background" /></label><Button className="h-11 sm:mb-0" disabled={busy !== null || !apiKey.trim()} onClick={() => void saveByok()}>{busy === "byok" ? <><Loader2 className="animate-spin" />Saving…</> : "Save API key"}</Button></div><details className="mt-4 border-t border-border pt-3 text-xs text-muted-foreground"><summary className="cursor-pointer select-none font-medium text-foreground">Provider and model settings</summary><div className="mt-3 grid gap-3 sm:grid-cols-2"><label className="grid gap-1">Endpoint<Input value={endpoint} onChange={(event) => setEndpoint(event.target.value)} className="h-10 rounded-lg bg-background" /></label><label className="grid gap-1">Chat model<Input value={chatModel} onChange={(event) => setChatModel(event.target.value)} className="h-10 rounded-lg bg-background" /></label><label className="grid gap-1 sm:col-span-2">Activity-processing model<Input value={processingModel} onChange={(event) => setProcessingModel(event.target.value)} className="h-10 rounded-lg bg-background" /></label></div></details></section>}
    {message && <p role="status" className="text-xs leading-5 text-muted-foreground">{message}</p>}
  </div>;
}

function ProviderChoice({ selected, disabled, title, description, onSelect }: { selected: boolean; disabled: boolean; title: string; description: string; onSelect: () => void }) {
  return <button type="button" role="radio" aria-checked={selected} disabled={disabled} onClick={onSelect} className={cn("flex min-h-24 items-start justify-between rounded-lg border px-4 py-3 text-left transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50", selected ? "border-primary bg-primary/[.05]" : "border-border hover:border-primary/45 hover:bg-muted/40")}><span><span className="block text-sm font-semibold">{title}</span><span className="mt-1 block text-xs leading-5 text-muted-foreground">{description}</span></span><span aria-hidden className={cn("mt-0.5 grid h-4 w-4 shrink-0 place-items-center rounded-full border", selected ? "border-primary" : "border-muted-foreground/60")}><span className={cn("h-2 w-2 rounded-full bg-primary transition", selected ? "scale-100" : "scale-0")} /></span></button>;
}

function SetupRow({ selected, disabled, title, description, onSelect, action, icon }: { selected: boolean; disabled: boolean; title: string; description: string; onSelect: () => void; action?: ReactNode; icon?: ReactNode }) {
  return <div className={cn("grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 px-1 py-1 transition", selected ? "bg-primary/[.04]" : disabled ? "" : "hover:bg-muted/50", disabled && !selected && "opacity-45")}><button type="button" disabled={disabled} onClick={onSelect} className="min-w-0 py-3 text-left disabled:cursor-not-allowed"><span className="flex items-center gap-2 text-sm font-semibold">{icon && <span className="text-primary [&_svg]:h-4 [&_svg]:w-4">{icon}</span>}{title}{selected && <Check className="h-4 w-4 text-primary" />}</span><span className="mt-1 block max-w-[60ch] text-xs leading-5 text-muted-foreground">{description}</span></button>{action && <span>{action}</span>}</div>;
}

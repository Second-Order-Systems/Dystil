"use client";

import { useCallback, useEffect, useMemo, useState, type FormEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "@/components/ui/use-toast";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { commands, type AiPresetView, type AiProviderStatusView } from "@/lib/utils/tauri";

type ManagedProvider = "codex" | "claude";
type ByokProvider = "anthropic" | "openai" | "openai_compatible" | "dystil_ai";

const apiProviderCopy: Record<ByokProvider, { label: string; presetName: string; defaultModel: string | null }> = {
  anthropic: { label: "Anthropic", presetName: "Anthropic API", defaultModel: "claude-haiku-4-5" },
  openai: { label: "OpenAI", presetName: "OpenAI API", defaultModel: "gpt-5.6-luna" },
  openai_compatible: { label: "OpenAI-compatible", presetName: "Custom API", defaultModel: null },
  dystil_ai: { label: "Dystil AI", presetName: "Dystil AI", defaultModel: "gpt-5.6-luna" },
};

const providerCopy: Record<ManagedProvider, { title: string; description: string }> = {
  codex: { title: "ChatGPT Plus or Pro", description: "Uses OpenAI’s official Codex client and your ChatGPT subscription." },
  claude: { title: "Claude Pro or Max", description: "Uses Anthropic’s official Claude Code client and your Claude subscription." },
};

function resultData<T>(result: { status: "ok"; data: T } | { status: "error"; error: string }): T {
  if (result.status === "error") throw new Error(result.error);
  return result.data;
}

function presetLabel(preset: AiPresetView) {
  if (preset.providerKind === "codex") return "ChatGPT subscription";
  if (preset.providerKind === "claude") return "Claude subscription";
  if (preset.providerKind === "anthropic") return "Anthropic API key";
  if (preset.providerKind === "openai") return "OpenAI API key";
  if (preset.providerKind === "dystil_ai") return "Dystil AI";
  if (preset.providerKind === "ollama") return "Ollama";
  return preset.name;
}

export function AiModelsSettings() {
  const [presets, setPresets] = useState<AiPresetView[]>([]);
  const [statuses, setStatuses] = useState<Partial<Record<ManagedProvider, AiProviderStatusView>>>({});
  const [ollamaModels, setOllamaModels] = useState<string[]>([]);
  const [ollamaModel, setOllamaModel] = useState("");
  const [ollamaDetail, setOllamaDetail] = useState("Checking for a running Ollama service…");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [claudeCode, setClaudeCode] = useState("");
  const [needsClaudeCode, setNeedsClaudeCode] = useState(false);
  const [showByok, setShowByok] = useState(false);
  const [byokProvider, setByokProvider] = useState<ByokProvider>("anthropic");
  const [byokName, setByokName] = useState("Anthropic API");
  const [byokEndpoint, setByokEndpoint] = useState("");
  const [byokModel, setByokModel] = useState("");
  const [byokKey, setByokKey] = useState("");

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [nextPresets, codex, claude] = await Promise.all([
        commands.aiPresetList().then(resultData),
        commands.aiProviderStatus("codex").then(resultData),
        commands.aiProviderStatus("claude").then(resultData),
      ]);
      setPresets(nextPresets);
      setStatuses({ codex, claude });
      setError("");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  const detectOllama = useCallback(async () => {
    try {
      const discovered = resultData(await commands.aiPresetDiscoverModels("ollama", null, null));
      setOllamaModels(discovered.models);
      setOllamaModel((current) => current || discovered.models[0] || "");
      setOllamaDetail(discovered.models.length ? `${discovered.models.length} installed ${discovered.models.length === 1 ? "model" : "models"} found.` : "Ollama is running, but it has no installed models.");
    } catch {
      setOllamaModels([]);
      setOllamaDetail("Ollama is not running on this computer.");
    }
  }, []);

  useEffect(() => { void load(); void detectOllama(); }, [detectOllama, load]);
  useEffect(() => {
    let dispose: (() => void) | undefined;
    listen("ai-provider-login-updated", () => void load()).then((next) => { dispose = next; }).catch(() => {});
    return () => dispose?.();
  }, [load]);

  const active = useMemo(() => presets.find((preset) => preset.active), [presets]);
  const customPresets = useMemo(() => presets.filter((preset) => ["anthropic", "openai", "openai_compatible", "dystil_ai"].includes(preset.providerKind)), [presets]);

  const run = async (operation: string, action: () => Promise<void>) => {
    setBusy(operation);
    setError("");
    try {
      await action();
      await load();
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause);
      setError(message);
      toast({ title: "AI model setup needs attention", description: message, variant: "destructive" });
    } finally {
      setBusy(null);
    }
  };

  const useManaged = (provider: ManagedProvider) => run(provider, async () => {
    let status = statuses[provider] ?? resultData(await commands.aiProviderStatus(provider));
    if (status.state !== "ready") status = resultData(await commands.aiProviderInstall(provider));
    resultData(await commands.aiPresetActivateManaged(provider, "default"));
    if (status.authenticated !== true) {
      const mode = resultData(await commands.aiProviderLogin(provider));
      if (provider === "claude" && mode === "codeRequired") setNeedsClaudeCode(true);
      toast({ title: `${providerCopy[provider].title} sign-in opened`, description: provider === "claude" ? "Finish in your browser, then paste the authorization code here." : "Finish signing in in your browser. Dystil will recognize it when you return." });
    }
  });

  const finishClaudeLogin = () => run("claude-code", async () => {
    resultData(await commands.aiProviderCompleteClaudeLogin(claudeCode));
    resultData(await commands.aiPresetActivateManaged("claude", "default"));
    setClaudeCode("");
    setNeedsClaudeCode(false);
  });

  const selectApiProvider = (provider: ByokProvider) => {
    setByokProvider(provider);
    setByokName(apiProviderCopy[provider].presetName);
    setByokEndpoint("");
    setByokModel(apiProviderCopy[provider].defaultModel ?? "");
  };

  const saveByok = (event: FormEvent) => {
    event.preventDefault();
    void run("byok", async () => {
      const saved = resultData(await commands.aiPresetSave(
        byokName.trim(),
        byokProvider,
        byokProvider === "openai_compatible" ? byokEndpoint.trim() : null,
        apiProviderCopy[byokProvider].defaultModel ?? byokModel.trim(),
        byokKey,
      ));
      try {
        resultData(await commands.aiPresetTest(saved.id));
      } catch (cause) {
        if (active) resultData(await commands.aiPresetActivate(active.id));
        resultData(await commands.aiPresetDelete(saved.id));
        throw cause;
      }
      setByokKey("");
      setShowByok(false);
      toast({ title: "API preset is ready", description: `${byokName.trim()} is now Dystil’s active AI model.` });
    });
  };

  const useOllama = () => run("ollama", async () => {
    const existing = presets.find((preset) => preset.providerKind === "ollama" && preset.model === ollamaModel);
    if (existing) resultData(await commands.aiPresetActivate(existing.id));
    else {
      const saved = resultData(await commands.aiPresetSave("Ollama", "ollama", null, ollamaModel, null));
      try {
        resultData(await commands.aiPresetTest(saved.id));
      } catch (cause) {
        if (active) resultData(await commands.aiPresetActivate(active.id));
        resultData(await commands.aiPresetDelete(saved.id));
        throw cause;
      }
    }
  });

  return <div className="mx-auto max-w-[880px]">
    <h1 className="text-[29px] font-medium tracking-[-0.02em]">AI models</h1>
    <p className="mb-[26px] mt-[8px] max-w-[700px] text-[18px] leading-[1.6] text-[#60636b]">Choose what Dystil uses to reason over your local work history.</p>

    <section aria-labelledby="active-runtime" className="mb-[26px]">
      <h2 id="active-runtime" className="mb-[10px] text-[14px] text-[#74777e]">Current setup</h2>
      <div className="rounded-[12px] border border-[#c9e7db] bg-[#f5faf7] px-[20px] py-[17px]">
        {loading ? <div className="h-12 animate-pulse rounded-[8px] bg-[#e3ece7]" /> : active ? <div className="flex items-center justify-between gap-6"><div className="min-w-0"><p className="text-[18px] font-medium text-[#173b31]">{presetLabel(active)}</p><p className="mt-1 truncate text-[14px] text-[#5d6f69]">{active.model === "default" ? "Provider-selected model" : active.model}{active.endpoint ? ` · ${active.endpoint}` : ""}</p></div><span className="flex shrink-0 items-center gap-2 text-[14px] font-medium text-[#0f6e56]"><span className="h-2 w-2 rounded-full bg-[#1d9e75]" />Active</span></div> : <div><p className="text-[18px] font-medium text-[#26312d]">No AI model selected</p><p className="mt-1 text-[14px] text-[#606b67]">Capture continues normally. Choose an option below before asking Dystil questions or generating findings.</p></div>}
      </div>
    </section>

    <section aria-labelledby="subscriptions" className="mb-[26px]">
      <div className="mb-[10px]"><h2 id="subscriptions" className="text-[18px] font-medium">Use a subscription</h2><p className="mt-1 text-[14px] text-[#74777e]">Dystil installs an isolated official client and uses the account you sign into.</p></div>
      <div className="divide-y divide-[#efefe9] overflow-hidden rounded-[12px] border border-[#e7e7e2] bg-white">
        {(["codex", "claude"] as const).map((provider) => {
          const status = statuses[provider];
          const isActive = active?.providerKind === provider;
          const ready = status?.state === "ready" && status.authenticated === true;
          const action = busy === provider ? "Preparing…" : isActive && ready ? "Active" : ready ? "Use" : status?.state === "notInstalled" ? "Set up" : "Sign in";
          return <div key={provider} className="flex min-h-[84px] items-center justify-between gap-6 px-[19px] py-[14px]"><div className="min-w-0"><p className="text-[17px] font-medium">{providerCopy[provider].title}</p><p className="mt-1 text-[14px] leading-5 text-[#74777e]">{providerCopy[provider].description}</p><p className={`mt-1 text-[13px] ${ready ? "text-[#0f6e56]" : "text-[#74777e]"}`}>{ready ? "Connected" : status?.state === "notInstalled" ? "Not installed" : status?.authenticated === false ? "Sign-in required" : "Checking connection…"}</p></div><button type="button" aria-label={`${action} ${providerCopy[provider].title}`} disabled={busy !== null || isActive && ready} onClick={() => void useManaged(provider)} className={secondaryButton}>{action}</button></div>;
        })}
        {needsClaudeCode && <div className="bg-[#fbfbfa] px-[19px] py-[16px]"><label htmlFor="claude-code" className="text-[14px] font-medium">Claude authorization code</label><div className="mt-2 flex gap-2"><input id="claude-code" value={claudeCode} onChange={(event) => setClaudeCode(event.target.value)} autoComplete="off" className={inputClass} placeholder="Paste the code shown by Claude" /><button type="button" disabled={!claudeCode.trim() || busy !== null} onClick={() => void finishClaudeLogin()} className={primaryButton}>{busy === "claude-code" ? "Connecting…" : "Connect"}</button></div></div>}
      </div>
    </section>

    <section aria-labelledby="local-runtime" className="mb-[26px]">
      <div className="mb-[10px]"><h2 id="local-runtime" className="text-[18px] font-medium">Run locally with Ollama</h2><p className="mt-1 text-[14px] text-[#74777e]">Requests stay on this computer. Dystil automatically checks the standard local Ollama address.</p></div>
      <div className="rounded-[12px] border border-[#e7e7e2] bg-white px-[19px] py-[16px]"><div className="flex items-center justify-between gap-6"><div><p className="text-[16px] font-medium">Ollama</p><p className={`mt-1 text-[14px] ${ollamaModels.length ? "text-[#0f6e56]" : "text-[#74777e]"}`}>{ollamaDetail}</p></div><button type="button" disabled={busy !== null} onClick={() => void detectOllama()} className={textButton}>Check again</button></div>{ollamaModels.length > 0 && <div className="mt-[15px] flex gap-2"><Select value={ollamaModel} onValueChange={setOllamaModel}><SelectTrigger aria-label="Ollama model" className={selectTriggerClass}><SelectValue placeholder="Choose an installed model" /></SelectTrigger><SelectContent position="popper" className={selectContentClass}>{ollamaModels.map((model) => <SelectItem key={model} value={model} className={selectItemClass}>{model}</SelectItem>)}</SelectContent></Select><button type="button" disabled={!ollamaModel || busy !== null || active?.providerKind === "ollama" && active.model === ollamaModel} onClick={() => void useOllama()} className={primaryButton}>{busy === "ollama" ? "Preparing…" : active?.providerKind === "ollama" && active.model === ollamaModel ? "Active" : "Use Ollama"}</button></div>}</div>
    </section>

    <section aria-labelledby="byok-runtime" className="mb-[26px]">
      <div className="mb-[10px] flex items-end justify-between gap-5"><div><h2 id="byok-runtime" className="text-[18px] font-medium">Use an API key</h2><p className="mt-1 text-[14px] text-[#74777e]">Keys are stored in your operating system’s credential store, never in Dystil’s database.</p></div>{!showByok && <button type="button" onClick={() => setShowByok(true)} className={textButton}>Add API preset</button>}</div>
      <div className="divide-y divide-[#efefe9] overflow-hidden rounded-[12px] border border-[#e7e7e2] bg-white">
        {customPresets.map((preset) => <div key={preset.id} className="flex min-h-[74px] items-center justify-between gap-5 px-[19px] py-[13px]"><div className="min-w-0"><p className="truncate text-[16px] font-medium">{preset.name}</p><p className="mt-1 truncate text-[13px] text-[#74777e]">{apiProviderCopy[preset.providerKind as ByokProvider]?.label ?? preset.endpoint} · {preset.model}</p></div><div className="flex shrink-0 items-center gap-4">{!preset.active && <button type="button" disabled={busy !== null} onClick={() => void run(`activate-${preset.id}`, async () => { resultData(await commands.aiPresetActivate(preset.id)); })} className={textButton}>Use</button>}<button type="button" disabled={busy !== null || preset.active} onClick={() => void run(`delete-${preset.id}`, async () => { resultData(await commands.aiPresetDelete(preset.id)); })} className="text-[14px] text-[#8b3f32] outline-none hover:underline focus-visible:ring-2 focus-visible:ring-[#8b3f32] disabled:cursor-not-allowed disabled:text-[#b8aaa7]">Remove</button></div></div>)}
        {customPresets.length === 0 && !showByok && <p className="px-[19px] py-[18px] text-[14px] text-[#74777e]">No API-key presets saved.</p>}
        {showByok && <form onSubmit={saveByok} className="bg-[#fbfbfa] px-[19px] py-[18px]">
          <div className="grid gap-[14px] sm:grid-cols-2">
            <Field label="Provider"><Select value={byokProvider} onValueChange={(value) => selectApiProvider(value as ByokProvider)}><SelectTrigger aria-label="API provider" className={selectTriggerClass}><SelectValue /></SelectTrigger><SelectContent position="popper" className={selectContentClass}>{(Object.entries(apiProviderCopy) as [ByokProvider, (typeof apiProviderCopy)[ByokProvider]][]).map(([value, copy]) => <SelectItem key={value} value={value} className={selectItemClass}>{copy.label}</SelectItem>)}</SelectContent></Select></Field>
            <Field label="Preset name"><input required maxLength={80} value={byokName} onChange={(event) => setByokName(event.target.value)} className={inputClass} /></Field>
            {byokProvider === "openai_compatible" && <Field label="API endpoint"><input required type="url" value={byokEndpoint} onChange={(event) => setByokEndpoint(event.target.value)} className={inputClass} placeholder="https://provider.example/v1" /></Field>}
            <Field label={byokProvider === "dystil_ai" ? "Dystil AI key" : "API key"}><input required type="password" autoComplete="new-password" value={byokKey} onChange={(event) => setByokKey(event.target.value)} className={inputClass} placeholder={byokProvider === "dystil_ai" ? "dst_live_…" : "Stored securely after saving"} /></Field>
            {byokProvider === "openai_compatible" && <Field label="Model"><input required maxLength={200} value={byokModel} onChange={(event) => setByokModel(event.target.value)} className={inputClass} placeholder="model ID" /></Field>}
          </div>
          <div className="mt-[16px] flex gap-4"><button disabled={busy !== null} className={primaryButton}>{busy === "byok" ? "Checking connection…" : "Save and use"}</button><button type="button" disabled={busy !== null} onClick={() => { setShowByok(false); setByokKey(""); }} className={textButton}>Cancel</button></div>
        </form>}
      </div>
    </section>

    {error && <p role="alert" className="mt-[18px] rounded-[9px] bg-[#fff4f1] px-[14px] py-[11px] text-[14px] text-[#8b3f32]">{error}</p>}
  </div>;
}

function Field({ label, children }: { label: string; children: React.ReactNode }) { return <label className="block"><span className="mb-2 block text-[13px] font-medium text-[#555b58]">{label}</span>{children}</label>; }

const inputClass = "h-[42px] min-w-0 w-full rounded-[8px] border border-[#d4d5d0] bg-white px-[12px] text-[14px] text-[#1a1c20] outline-none focus:border-[#0f6e56] focus:ring-1 focus:ring-[#0f6e56]";
const selectTriggerClass = "h-[42px] min-w-0 w-full rounded-[8px] border-[#d4d5d0] bg-white px-[12px] font-sans text-[14px] text-[#1a1c20] shadow-none outline-none focus:border-[#0f6e56] focus:ring-1 focus:ring-[#0f6e56]";
const selectContentClass = "z-[100] max-h-[280px] rounded-[10px] border-[#d4d5d0] bg-white p-1 font-sans text-[#1a1c20] shadow-[0_12px_32px_rgba(25,35,31,0.16)]";
const selectItemClass = "min-h-[38px] rounded-[7px] py-2 pl-8 pr-3 font-sans text-[14px] focus:bg-[#e7f4ee] focus:text-[#0b5b47]";
const primaryButton = "h-[42px] shrink-0 rounded-[8px] bg-[#0f6e56] px-[16px] text-[14px] font-medium text-white outline-none hover:bg-[#0b5b47] focus-visible:ring-2 focus-visible:ring-[#0f6e56] focus-visible:ring-offset-2 disabled:cursor-wait disabled:bg-[#a8bdb5]";
const secondaryButton = "h-[38px] shrink-0 rounded-[8px] border border-[#cfd4d0] bg-white px-[14px] text-[14px] font-medium text-[#27302c] outline-none hover:border-[#8fa99f] hover:bg-[#f5f8f6] focus-visible:ring-2 focus-visible:ring-[#0f6e56] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:bg-[#f3f4f2] disabled:text-[#8b918e]";
const textButton = "shrink-0 text-[14px] font-medium text-[#0f6e56] outline-none hover:underline focus-visible:ring-2 focus-visible:ring-[#0f6e56] focus-visible:ring-offset-2 disabled:cursor-wait disabled:text-[#8fa99f]";

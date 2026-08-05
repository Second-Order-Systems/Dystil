"use client";

import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "@/components/ui/use-toast";
import { Slider } from "@/components/ui/slider";
import { useSettings } from "@/lib/hooks/use-settings";
import { getBuildCapabilities } from "@/lib/build-capabilities";
import { requestPermissionWithFlow } from "@/lib/utils/permission-flow";
import { commands, type WhenItRunsView } from "@/lib/utils/tauri";
import { TextAction } from "./page-primitives";
import { CAPTURE_CATEGORY_COPY } from "./capture-categories";
import { AiModelsSettings } from "./ai-models-settings";
import type { DystilShellProps, SettingsTab } from "./types";

const settingsTabs: readonly SettingsTab[] = ["What Dystil can see", "AI models", "When it runs", "Storage", "Notifications", "Invite your team", "About"];
const DYSTIL_SOURCE_URL = "https://github.com/Second-Order-Systems/Dystil";

export function SettingsWorkspace(props: DystilShellProps & { initialTab?: SettingsTab; onBack: () => void }) {
  const [tab, setTab] = useState<SettingsTab>(props.initialTab ?? "What Dystil can see");
  const [update, setUpdate] = useState<AppUpdateSettings | null>(null);
  const [installingUpdate, setInstallingUpdate] = useState(false);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    invoke<AppUpdateSettings>("get_app_update_settings").then((value) => { if (!cancelled) setUpdate(value); }).catch(() => {});
    listen<AppUpdateSettings>("app-update-state-changed", ({ payload }) => setUpdate(payload)).then((dispose) => { unlisten = dispose; }).catch(() => {});
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  const installAvailableUpdate = async () => {
    setInstallingUpdate(true);
    try {
      await invoke("install_app_update");
    } catch (cause) {
      setInstallingUpdate(false);
      toast({ title: "Could not install the update", description: cause instanceof Error ? cause.message : String(cause), variant: "destructive" });
    }
  };

  const showUpdateBanner = Boolean(update?.updaterAvailable && !update.autoUpdate && update.availableVersion);
  return <div className="grid h-full grid-cols-[268px_minmax(0,1fr)]">
    <aside className="flex min-h-0 flex-col border-r border-[#e3e3de] bg-white py-[27px]">
      <button type="button" onClick={props.onBack} className="px-[27px] text-left text-[17px] text-[#60636b] outline-none hover:text-[#0f6e56] focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[#0f6e56]">←&nbsp; Back to Dystil</button>
      <p className="mb-[10px] mt-[28px] px-[27px] text-[13px] tracking-[0.08em] text-[#9a9da5]">SETTINGS</p>
      <nav>{settingsTabs.map((item) => <button key={item} type="button" aria-current={tab === item ? "page" : undefined} onClick={() => setTab(item)} className={`relative block min-h-[47px] w-full px-[27px] text-left text-[17px] outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[#0f6e56] ${tab === item ? "bg-[#e1f5ee] text-[#0f6e56]" : "text-[#60636b] hover:bg-[#f8f8f7]"}`}>{tab === item && <span aria-hidden="true" className="absolute inset-y-0 left-0 w-[2px] bg-[#1d9e75]" />}{item}</button>)}</nav>
      {showUpdateBanner && <div className="mt-auto px-[16px] pt-6">
        <div className="rounded-[11px] border border-[#c9e7db] bg-[#f3f8f5] p-[14px]">
          <p className="text-[15px] font-medium text-[#174d3f]">Dystil {update?.availableVersion} is ready</p>
          <p className="mt-1 text-[13px] leading-[1.45] text-[#4f685f]">Update when you’re ready.</p>
          <button type="button" disabled={installingUpdate} onClick={() => void installAvailableUpdate()} className="mt-3 w-full rounded-[8px] bg-[#176f59] px-3 py-2 text-[14px] font-medium text-white outline-none hover:bg-[#0f5b48] focus-visible:ring-2 focus-visible:ring-[#0f6e56] focus-visible:ring-offset-2 disabled:cursor-wait disabled:opacity-60">{installingUpdate ? "Installing…" : "Update now"}</button>
        </div>
      </div>}
    </aside>
    <section className="min-h-0 overflow-y-auto px-[48px] py-[42px]"><SettingsPane tab={tab} {...props} /></section>
  </div>;
}

function SettingsPane({ tab, userName, userEmail, onLogout, loggingOut, version }: DystilShellProps & { tab: SettingsTab }) {
  if (tab === "What Dystil can see") return <CaptureVisibilitySettings />;
  if (tab === "AI models") return <AiModelsSettings />;
  if (tab === "When it runs") return <WhenItRunsSettings />;
  if (tab === "Storage") return <StorageSettings />;
  if (tab === "Notifications") return <SettingsPage title="Notifications" lede="Dystil interrupts rarely and only when it has something. There are no reminders, streaks, or nudges to come back."><SettingsCard><ToggleRow title="When it finds something worth telling you" description="Roughly once a week, often less" initial /><ToggleRow title="When something you asked for is ready" description="" initial /></SettingsCard></SettingsPage>;
  if (tab === "Invite your team") return <InviteTeamSettings />;
  return <AboutSettings userName={userName} userEmail={userEmail} onLogout={onLogout} loggingOut={loggingOut} version={version} />;
}

type AppUpdateSettings = { autoUpdate: boolean; updaterAvailable: boolean; availableVersion?: string | null };

export function AboutSettings({ userName, userEmail, onLogout, loggingOut, version }: Pick<DystilShellProps, "userName" | "userEmail" | "onLogout" | "loggingOut" | "version">) {
  const [updateSettings, setUpdateSettings] = useState<AppUpdateSettings | null>(null);
  const [updateError, setUpdateError] = useState("");
  const [savingAutoUpdate, setSavingAutoUpdate] = useState(false);
  const [installingUpdate, setInstallingUpdate] = useState(false);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    invoke<AppUpdateSettings>("get_app_update_settings")
      .then((result) => { if (!cancelled) setUpdateSettings(result); })
      .catch((cause) => {
        if (cancelled) return;
        setUpdateError(cause instanceof Error ? cause.message : String(cause));
      });
    listen<AppUpdateSettings>("app-update-state-changed", ({ payload }) => setUpdateSettings(payload)).then((dispose) => { unlisten = dispose; }).catch(() => {});
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  const setAutoUpdate = async (enabled: boolean) => {
    setSavingAutoUpdate(true);
    setUpdateError("");
    try {
      const result = await invoke<AppUpdateSettings>("set_app_auto_update", { enabled });
      setUpdateSettings(result);
    } catch (cause) {
      setUpdateError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSavingAutoUpdate(false);
    }
  };

  const installUpdate = async () => {
    setInstallingUpdate(true);
    setUpdateError("");
    try {
      await invoke("install_app_update");
    } catch (cause) {
      setUpdateError(cause instanceof Error ? cause.message : String(cause));
      setInstallingUpdate(false);
    }
  };

  const openSource = async () => {
    try {
      await openUrl(DYSTIL_SOURCE_URL);
    } catch (cause) {
      toast({ title: "Could not open the source code", description: cause instanceof Error ? cause.message : String(cause), variant: "destructive" });
    }
  };

  const updateAction = updateSettings?.updaterAvailable && !updateSettings.autoUpdate && updateSettings.availableVersion
    ? installingUpdate
      ? <span role="status" className="text-[14px] text-[#74777e]">Installing…</span>
      : <button type="button" onClick={() => void installUpdate()} className="text-[15px] font-medium text-[#0f6e56] outline-none hover:text-[#094b3b] focus-visible:ring-2 focus-visible:ring-[#0f6e56] focus-visible:ring-offset-2">Update to {updateSettings.availableVersion}</button>
    : null;

  return <SettingsPage title="About" lede="">
    <SettingsCard>
      <div className="flex min-h-[62px] items-center justify-between gap-5 border-b border-[#efefe9] px-[19px] py-[13px]"><span className="text-[17px]">Version {version || "—"}</span>{updateAction}</div>
      {updateSettings?.updaterAvailable && <div className="flex min-h-[72px] items-center justify-between gap-5 border-b border-[#efefe9] px-[19px] py-[13px]"><div><p className="text-[17px]">Update automatically</p><p className="mt-1 text-[14px] text-[#74777e]">{updateSettings.autoUpdate ? "Installs new versions automatically and restarts Dystil to finish." : "Dystil will tell you when an update is ready."}</p></div><ToggleSwitch label="Update Dystil automatically" checked={updateSettings.autoUpdate} disabled={savingAutoUpdate} onChange={(enabled) => void setAutoUpdate(enabled)} /></div>}
      {updateSettings && !updateSettings.updaterAvailable && <div className="border-b border-[#efefe9] px-[19px] py-[13px]"><p className="text-[17px]">Updates</p><p className="mt-1 text-[14px] text-[#74777e]">Managed outside Dystil in this build.</p></div>}
      {updateError && <p role="alert" className="border-b border-[#efefe9] bg-[#fff5f2] px-[19px] py-[10px] text-[13px] text-[#8b3f32]">{updateError}</p>}
      <div className="flex min-h-[62px] items-center justify-between gap-5 px-[19px] py-[13px]"><span className="text-[17px]">Read the source code</span><button type="button" onClick={() => void openSource()} className="text-[15px] text-[#0f6e56] outline-none hover:text-[#094b3b] focus-visible:ring-2 focus-visible:ring-[#0f6e56] focus-visible:ring-offset-2">Open</button></div>
    </SettingsCard>
    <SettingsCard padded><div className="flex justify-between gap-5"><span className="min-w-0 truncate text-[16px]" title={userEmail || userName}>{userEmail || userName}</span><button type="button" onClick={onLogout} disabled={loggingOut} className="shrink-0 text-[15px] text-[#60636b] outline-none hover:text-[#1a1c20] focus-visible:ring-2 focus-visible:ring-[#0f6e56] focus-visible:ring-offset-2 disabled:opacity-50">{loggingOut ? "Signing out…" : "Sign out"}</button></div></SettingsCard>
  </SettingsPage>;
}

const DYSTIL_DOWNLOAD_URL = "https://2os.ai/download";

export function InviteTeamSettings() {
  const [copying, setCopying] = useState(false);
  const [copied, setCopied] = useState(false);

  const copyDownloadLink = async () => {
    setCopying(true);
    try {
      const result = await commands.copyTextToClipboard(DYSTIL_DOWNLOAD_URL);
      if (result.status === "error") throw new Error(result.error);
      setCopied(true);
      toast({ title: "Dystil link copied", description: "It is ready to send to your team." });
    } catch (cause) {
      setCopied(false);
      toast({
        title: "Could not copy the Dystil link",
        description: cause instanceof Error ? cause.message : String(cause),
        variant: "destructive",
      });
    } finally {
      setCopying(false);
    }
  };

  return <SettingsPage title="Invite your team" lede="Think someone on your team would find Dystil useful? Send them a link.">
    <SettingsCard padded accent>
      <h2 className="text-[19px] font-medium tracking-[-0.01em]">Share Dystil</h2>
      <p className="mt-[7px] max-w-[650px] text-[16px] leading-[1.6] text-[#60636b]">They’ll download their own copy of Dystil. Your accounts and data are never connected.</p>
      <button
        type="button"
        onClick={() => void copyDownloadLink()}
        disabled={copying}
        className="mt-[20px] min-w-[162px] rounded-[9px] bg-[#0f6e56] px-[18px] py-[11px] text-[16px] font-medium text-white transition-colors hover:bg-[#0b5b47] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#0f6e56] disabled:cursor-wait disabled:bg-[#9ab9ae]"
      >
        {copying ? "Copying…" : copied ? "Link copied" : "Copy Dystil link"}
      </button>
      <p className="sr-only" aria-live="polite">{copied ? "Dystil download link copied to the clipboard." : ""}</p>
    </SettingsCard>
  </SettingsPage>;
}

export function StorageSettings() {
  const { getDataDir } = useSettings();
  const [storage, setStorage] = useState<RetentionStorageView | null>(null);
  const [selectedDays, setSelectedDays] = useState(90);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [opening, setOpening] = useState(false);

  const load = async (forceRefresh = false) => {
    setIsLoading(true);
    try {
      const result = await invoke<RetentionStorageView>("get_retention_storage", { forceRefresh });
      setStorage(result);
      setSelectedDays(result.retentionDays);
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => { void load(true); }, []);

  const saveRetention = async () => {
    setSaving(true);
    try {
      const result = await invoke<RetentionStorageView>("set_retention_days", { retentionDays: selectedDays });
      setStorage(result);
      setSelectedDays(result.retentionDays);
      setError(null);
      toast({ title: "Retention updated", description: selectedDays === 0 ? "Dystil will keep raw work history until you change this setting." : `Raw work history will be kept for ${retentionLabel(selectedDays).toLowerCase()}.` });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(false);
    }
  };

  const openDataFolder = async () => {
    setOpening(true);
    try {
      await openPath(await getDataDir());
    } catch (cause) {
      toast({
        title: "Could not open the Dystil data folder",
        description: cause instanceof Error ? cause.message : String(cause),
        variant: "destructive",
      });
    } finally {
      setOpening(false);
    }
  };

  const changed = storage !== null && selectedDays !== storage.retentionDays;
  const selectedIndex = Math.max(0, RETENTION_OPTIONS.findIndex((option) => option.days === selectedDays));
  const shortening = storage !== null && selectedDays > 0 && (storage.retentionDays === 0 || selectedDays < storage.retentionDays);
  const projectedBytes = storage && selectedDays > 0 ? storage.fixedBytes + storage.dailyHistoryBytes * selectedDays : null;
  const exceedsDeviceCapacity = storage !== null && projectedBytes !== null && projectedBytes > storage.totalDataBytes + storage.availableSpaceBytes;

  return <SettingsPage title="Storage" lede="How much space Dystil uses, and how far back it can remember.">
    <SettingsCard padded>
      {isLoading && !storage && <SettingsLoadingRows count={1} />}
      {error && !storage && <SettingsInlineError message="Dystil could not measure local storage." onRetry={() => void load(true)} />}
      {storage && <>
        <div className="flex items-center justify-between gap-8">
          <div className="min-w-0">
            <p className="text-[25px] font-medium">{storage.totalDataSize} used</p>
            <p className="mt-1 text-[14px] text-[#60636b]">{storage.availableSpace} free on this device</p>
          </div>
          <SmallButton onClick={() => void load(true)} disabled={isLoading}>{isLoading ? "Refreshing…" : "Refresh"}</SmallButton>
        </div>
        {error && <p role="alert" className="mt-[18px] text-[14px] text-[#8b3f32]">Could not refresh storage. Showing the previous result.</p>}
      </>}
    </SettingsCard>
    {storage && <SettingsCard padded>
      <div className="flex items-baseline justify-between gap-6">
        <p id="retention-slider-label" className="text-[18px] font-medium">Keep raw work history for</p>
        <span className="shrink-0 text-[17px] font-medium text-[#0f6e56]">{retentionLabel(selectedDays)}</span>
      </div>
      <p className="mt-1 max-w-[650px] text-[15px] leading-[1.5] text-[#60636b]">Findings remain available. Screenshots, captured text, interface events, and their search entries expire together.</p>
      <div className="mt-[24px] px-[4px]">
        <Slider thumbAriaLabelledby="retention-slider-label" thumbAriaValuetext={retentionLabel(selectedDays)} min={0} max={RETENTION_OPTIONS.length - 1} step={1} value={[selectedIndex]} disabled={saving} onValueChange={([index]) => setSelectedDays(RETENTION_OPTIONS[index]?.days ?? selectedDays)} />
        <div className="relative mt-[9px] h-[34px]">
          {RETENTION_OPTIONS.map((option, index) => <button key={option.days} type="button" aria-pressed={selectedDays === option.days} disabled={saving} onClick={() => setSelectedDays(option.days)} className={`absolute top-0 whitespace-nowrap rounded-[4px] px-[2px] text-[13px] outline-none transition-colors focus-visible:ring-2 focus-visible:ring-[#0f6e56] disabled:opacity-50 ${selectedDays === option.days ? "font-medium text-[#0f6e56]" : "text-[#74777e] hover:text-[#1a1c20]"}`} style={{ left: `${index * 20}%`, transform: index === 0 ? "none" : index === RETENTION_OPTIONS.length - 1 ? "translateX(-100%)" : "translateX(-50%)" }}>{option.label}</button>)}
        </div>
      </div>
      <div className="mt-[8px] rounded-[10px] bg-[#f2f8f5] px-[16px] py-[14px]">
        <p className="text-[17px] font-medium text-[#0f6e56]">{retentionMessage(selectedDays).title}</p>
        <p className="mt-1 text-[15px] leading-[1.55] text-[#60636b]">{retentionMessage(selectedDays).body}</p>
        <p className="mt-2 text-[14px] text-[#74777e]">{storage.dailyHistoryBytes === 0 ? "Not enough captured history to estimate growth yet." : selectedDays === 0 ? `Growing about ${formatBytes(storage.dailyHistoryBytes * 30)} a month at your current rate.` : `${storage.estimateIsEarly ? "Early estimate" : "Estimate"}: about ${formatBytes(projectedBytes ?? 0)} total at your current rate.`}</p>
        {exceedsDeviceCapacity && <p className="mt-2 text-[14px] font-medium text-[#8b3f32]">This device does not currently have enough free space for that projection.</p>}
      </div>
      {changed && <div className="mt-[18px] flex items-center justify-between gap-6 border-t border-[#efefe9] pt-[18px]">
        <p className={`text-[14px] leading-[1.5] ${shortening ? "text-[#8b3f32]" : "text-[#60636b]"}`}>{shortening ? `Applying this permanently deletes raw history older than ${retentionLabel(selectedDays).toLowerCase()}.` : "The new policy takes effect immediately."}</p>
        <button type="button" onClick={() => void saveRetention()} disabled={saving} className={`shrink-0 rounded-[9px] px-[15px] py-[9px] text-[15px] font-medium text-white outline-none focus-visible:ring-2 focus-visible:ring-offset-2 disabled:opacity-50 ${shortening ? "bg-[#9b4336] focus-visible:ring-[#8b3f32]" : "bg-[#0f6e56] focus-visible:ring-[#0f6e56]"}`}>{saving ? "Applying…" : shortening ? "Delete older history and apply" : "Apply"}</button>
      </div>}
    </SettingsCard>}
    <button type="button" onClick={() => void openDataFolder()} disabled={opening} className="mt-[2px] text-[15px] font-medium text-[#0f6e56] underline decoration-[#8bcdb6] underline-offset-2 outline-none hover:decoration-[#0f6e56] focus-visible:ring-2 focus-visible:ring-[#0f6e56] disabled:opacity-50">{opening ? "Opening…" : "Open data folder"}</button>
  </SettingsPage>;
}

type RetentionStorageView = {
  retentionDays: number;
  totalDataSize: string;
  totalDataBytes: number;
  availableSpace: string;
  availableSpaceBytes: number;
  fixedBytes: number;
  dailyHistoryBytes: number;
  observedDays: number;
  estimateIsEarly: boolean;
};

const RETENTION_OPTIONS = [
  { days: 7, label: "1 week" },
  { days: 14, label: "2 weeks" },
  { days: 30, label: "1 month" },
  { days: 90, label: "3 months" },
  { days: 365, label: "1 year" },
  { days: 0, label: "Forever" },
] as const;

function retentionLabel(days: number) {
  return RETENTION_OPTIONS.find((option) => option.days === days)?.label ?? `${days} days`;
}

function formatBytes(bytes: number) {
  const units = ["KB", "MB", "GB", "TB"];
  let value = Math.max(0, bytes) / 1024;
  let unit = units[0];
  for (let index = 1; value >= 1024 && index < units.length; index += 1) {
    value /= 1024;
    unit = units[index];
  }
  return `${new Intl.NumberFormat(undefined, { maximumFractionDigits: value >= 10 ? 0 : 1 }).format(value)} ${unit}`;
}

function retentionMessage(days: number) {
  if (days === 7) return { title: "Recent work", body: "Enough for daily and weekly patterns. Monthly and quarterly routines may not repeat before their source history expires." };
  if (days === 14) return { title: "Daily and weekly patterns", body: "Enough to recognize recurring work within a couple of weeks. Monthly routines may still be missed." };
  if (days === 30) return { title: "About a month of your work", body: "Enough for weekly and month-end patterns. Quarterly routines may not repeat often enough." };
  if (days === 90) return { title: "About a quarter of your work", body: "Recommended. Long enough to see month-end and quarterly cycles without keeping raw history indefinitely." };
  if (days === 365) return { title: "About a year of your work", body: "Enough to see seasonal and annual patterns as well as weekly, monthly, and quarterly routines." };
  return { title: "Keep everything", body: "Dystil will not automatically delete raw work history. You can shorten retention later." };
}

type CaptureCategory = { id: string; enabled: boolean };
type CaptureSource = { id: string; kind: "app" | "site"; name: string; activeMinutes: number; enabled: boolean };
type CaptureVisibility = { categories: CaptureCategory[]; sources: CaptureSource[]; sourcesError?: string | null };

function pauseStatusCopy(pauseUntil?: string | null) {
  if (!pauseUntil) return "Paused until you resume";
  const deadline = new Date(pauseUntil);
  if (Number.isNaN(deadline.getTime())) return "Paused";
  const now = new Date();
  const tomorrow = new Date(now);
  tomorrow.setDate(now.getDate() + 1);
  const sameDay = deadline.toDateString() === now.toDateString();
  const nextDay = deadline.toDateString() === tomorrow.toDateString();
  const day = sameDay ? "today" : nextDay ? "tomorrow" : deadline.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  return `Paused · resumes ${day} at ${deadline.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}`;
}

export function WhenItRunsSettings() {
  const { reloadStore } = useSettings();
  const [runtime, setRuntime] = useState<WhenItRunsView | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [enterpriseManaged, setEnterpriseManaged] = useState<boolean | null>(null);

  const load = async () => {
    try {
      const result = await commands.getWhenItRunsSettings();
      if (result.status === "error") throw new Error(result.error);
      setRuntime(result.data);
      setLoadError(false);
    } catch {
      setLoadError(true);
    }
  };

  useEffect(() => {
    void load();
    void getBuildCapabilities().then((capabilities) => {
      setEnterpriseManaged(capabilities.enterpriseManaged);
    });
    let unlisten: (() => void) | undefined;
    listen("recording-status-changed", () => void load())
      .then((dispose) => { unlisten = dispose; })
      .catch(() => {});
    return () => unlisten?.();
  }, []);

  const run = async (operation: string, title: string, action: () => Promise<void>) => {
    setBusy(operation);
    try {
      await action();
      await reloadStore();
      await load();
    } catch (error) {
      await load();
      toast({ title, description: error instanceof Error ? error.message : String(error), variant: "destructive" });
    } finally {
      setBusy(null);
    }
  };

  const setScreenshots = async (enabled: boolean) => {
    await run("screenshots", "Could not update screenshot capture", async () => {
      if (enabled) {
        let permission = await commands.checkScreenRecordingPermission();
        if (permission !== "granted" && permission !== "notNeeded") {
          await requestPermissionWithFlow("screenRecording");
          permission = await commands.checkScreenRecordingPermission();
        }
        if (permission !== "granted" && permission !== "notNeeded") {
          throw new Error("Screen Recording permission was not granted.");
        }
      }
      const result = await commands.setScreenshotCaptureEnabled(enabled);
      if (result.status === "error") throw new Error(result.error);
    });
  };

  return <SettingsPage title="When it runs" lede="Dystil works in the background whether or not this window is open. If it is not running while you work, it has nothing to tell you later.">
    <SettingsCard>
      {!runtime && !loadError && <SettingsLoadingRows count={2} />}
      {runtime && <>
        <CaptureToggleRow title="Start when you log in" description="Starts quietly in the system tray without opening a window." checked={runtime.autostartEnabled} busy={busy !== null} switchLabel={`${runtime.autostartEnabled ? "Stop" : "Start"} Dystil when you log in`} onChange={(enabled) => void run("autostart", "Could not update login startup", async () => { const result = await commands.setAutostart(enabled); if (result.status === "error") throw new Error(result.error); })} />
        <CaptureToggleRow
          title="Capture screenshots"
          description={enterpriseManaged
            ? "Required and managed by your organization."
            : "Adds visual context. Turning this off keeps accessibility and text capture running."}
          checked={runtime.screenshotEnabled}
          busy={busy !== null || enterpriseManaged !== false}
          switchLabel={enterpriseManaged
            ? "Screenshot capture is managed by your organization"
            : `${runtime.screenshotEnabled ? "Stop" : "Start"} capturing screenshots`}
          onChange={(enabled) => void setScreenshots(enabled)}
        />
      </>}
      {loadError && <SettingsInlineError message="Dystil could not load its runtime settings." onRetry={() => void load()} />}
    </SettingsCard>
    <SettingsCard padded>
      <div className="flex items-center justify-between gap-5">
        <div className="min-w-0">
          <h2 className="text-[18px] font-medium">{runtime?.capturePaused ? "Capture paused" : runtime?.captureRunning ? "Pause capture" : "Capture is not running"}</h2>
          <p className="mt-1 text-[16px] text-[#60636b]">{runtime?.capturePaused ? pauseStatusCopy(runtime.pauseUntil) : runtime?.captureRunning ? "Pause here or from the system tray." : "Resume to keep building your local work history."}</p>
        </div>
        {runtime && <div className="flex shrink-0 gap-2">
          {runtime.captureRunning ? <>
            <SmallButton onClick={() => void run("pause", "Could not pause capture", async () => { const result = await commands.pauseCaptureFor("oneHour"); if (result.status === "error") throw new Error(result.error); })} disabled={busy !== null}>{busy === "pause" ? "Pausing…" : "1 hour"}</SmallButton>
            <SmallButton onClick={() => void run("pause", "Could not pause capture", async () => { const result = await commands.pauseCaptureFor("today"); if (result.status === "error") throw new Error(result.error); })} disabled={busy !== null}>Today</SmallButton>
          </> : <SmallButton onClick={() => void run("resume", "Could not resume capture", async () => { const result = await commands.resumeCaptureNow(); if (result.status === "error") throw new Error(result.error); })} disabled={busy !== null}>{busy === "resume" ? "Resuming…" : "Resume now"}</SmallButton>}
        </div>}
      </div>
    </SettingsCard>
  </SettingsPage>;
}

function CaptureVisibilitySettings() {
  const { reloadStore } = useSettings();
  const [visibility, setVisibility] = useState<CaptureVisibility | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState<Set<string>>(() => new Set());
  const [openGroups, setOpenGroups] = useState<Set<string>>(() => new Set(["primary", "disabled"]));

  const load = async () => {
    setLoadError(null);
    try {
      setVisibility(await invoke<CaptureVisibility>("get_capture_visibility"));
    } catch (error) {
      setLoadError(error instanceof Error ? error.message : String(error));
    }
  };

  useEffect(() => { void load(); }, []);

  const setBusyState = (id: string, value: boolean) => setBusy((current) => {
    const next = new Set(current);
    if (value) next.add(id); else next.delete(id);
    return next;
  });

  const changeCategory = async (category: CaptureCategory, enabled: boolean) => {
    if (!visibility) return;
    setBusyState(category.id, true);
    setVisibility({ ...visibility, categories: visibility.categories.map((item) => item.id === category.id ? { ...item, enabled } : item) });
    try {
      await invoke("set_capture_category_enabled", { categoryId: category.id, enabled });
      await reloadStore();
      await load();
    } catch (error) {
      setVisibility((current) => current ? { ...current, categories: current.categories.map((item) => item.id === category.id ? category : item) } : current);
      toast({ title: "Could not update what Dystil can see", description: error instanceof Error ? error.message : String(error), variant: "destructive" });
    } finally {
      setBusyState(category.id, false);
    }
  };

  const changeSource = async (source: CaptureSource, enabled: boolean) => {
    if (!visibility) return;
    setBusyState(source.id, true);
    setVisibility({ ...visibility, sources: visibility.sources.map((item) => item.id === source.id ? { ...item, enabled } : item) });
    try {
      await invoke("set_capture_source_enabled", { sourceKind: source.kind, sourceName: source.name, enabled });
      await reloadStore();
      await load();
    } catch (error) {
      setVisibility((current) => current ? { ...current, sources: current.sources.map((item) => item.id === source.id ? source : item) } : current);
      toast({ title: `Could not ${enabled ? "allow" : "block"} ${source.name}`, description: error instanceof Error ? error.message : String(error), variant: "destructive" });
    } finally {
      setBusyState(source.id, false);
    }
  };

  const groups = useMemo(() => {
    const filtered = (visibility?.sources ?? []).filter((source) => source.name.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
    const enabled = filtered.filter((source) => source.enabled);
    return [
      { id: "primary", title: "Where most of your work happens", description: "Worth a look. These shape almost every finding.", items: enabled.slice(0, 6) },
      { id: "occasional", title: "Now and then", description: "", items: enabled.slice(6, 14) },
      { id: "rare", title: "Barely touched", description: "", items: enabled.slice(14) },
      { id: "disabled", title: "Turned off by you", description: "Dystil skips these from now on.", items: filtered.filter((source) => !source.enabled) },
    ].filter((group) => group.items.length > 0);
  }, [query, visibility?.sources]);

  const toggleGroup = (id: string) => setOpenGroups((current) => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  });
  const turnedOff = visibility?.sources.filter((source) => !source.enabled).length ?? 0;

  return <SettingsPage title="What Dystil can see" lede={<>Changes here apply from now on. To remove something already captured, use <a href="/home/privacy" className="font-medium text-[#0f6e56] underline decoration-[#8bcdb6] underline-offset-2 outline-none hover:decoration-[#0f6e56] focus-visible:ring-2 focus-visible:ring-[#0f6e56]">What stays on this computer</a>.</>}>
    <SettingLabel>Sensitive categories</SettingLabel>
    <SettingsCard>
      {!visibility && !loadError ? <SettingsLoadingRows count={5} /> : visibility?.categories.map((category) => {
        const copy = CAPTURE_CATEGORY_COPY[category.id] ?? { title: category.id, description: "" };
        return <CaptureToggleRow key={category.id} title={copy.title} description={copy.description} checked={category.enabled} busy={busy.size > 0} onChange={(enabled) => void changeCategory(category, enabled)} />;
      })}
      {loadError && <SettingsInlineError message="Dystil could not load your capture filters." onRetry={() => void load()} />}
    </SettingsCard>

    <div className="mb-[10px] mt-[28px] flex items-baseline justify-between gap-4">
      <SettingLabel className="mb-0">Apps and sites it has come across</SettingLabel>
      {visibility && <span className="text-[14px] text-[#60636b]">{turnedOff ? `${turnedOff} turned off` : "All allowed"}</span>}
    </div>
    <SettingsCard>
      <label className="block border-b border-[#efefe9] px-[17px] py-[12px]">
        <span className="sr-only">Find an app or site</span>
        <input value={query} onChange={(event) => setQuery(event.target.value)} className="w-full bg-transparent text-[15px] text-[#1a1c20] outline-none placeholder:text-[#74777e] focus-visible:ring-0" type="search" placeholder="Find an app or site" />
      </label>
      {!visibility && !loadError && <SettingsLoadingRows count={4} />}
      {visibility?.sourcesError && <SettingsInlineError message="Recent apps and sites are unavailable while the capture database is offline." onRetry={() => void load()} />}
      {visibility && !visibility.sourcesError && groups.map((group) => {
        const expanded = query.trim().length > 0 || openGroups.has(group.id);
        return <div key={group.id}>
          <button type="button" aria-expanded={expanded} onClick={() => toggleGroup(group.id)} className="flex w-full items-center justify-between gap-5 border-b border-[#efefe9] bg-[#f8f8f7] px-[18px] py-[13px] text-left outline-none hover:bg-[#f2f2ef] focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[#1d9e75]">
            <span className="min-w-0"><span className="block text-[16px] font-medium">{group.title}</span>{expanded && group.description && <span className="mt-1 block text-[14px] text-[#74777e]">{group.description}</span>}</span>
            <span className="shrink-0 text-[14px] text-[#60636b]">{group.items.length}{!expanded && " · show"}</span>
          </button>
          {expanded && group.items.map((source) => <CaptureToggleRow key={source.id} title={source.name} description={formatActiveTime(source.activeMinutes)} checked={source.enabled} busy={busy.size > 0} onChange={(enabled) => void changeSource(source, enabled)} muted={!source.enabled} />)}
        </div>;
      })}
      {visibility && !visibility.sourcesError && groups.length === 0 && <div className="px-[18px] py-[22px] text-[15px] text-[#60636b]">{query ? "No apps or sites match that search." : "No apps or sites have been captured this month yet."}</div>}
      {loadError && <SettingsInlineError message="Dystil could not load the apps and sites it has seen." onRetry={() => void load()} />}
    </SettingsCard>
    <p className="mt-[14px] max-w-[690px] text-[14px] leading-[1.6] text-[#60636b]">New apps and sites are allowed by default. Turning one off affects future capture only; existing local history is unchanged.</p>
  </SettingsPage>;
}

function formatActiveTime(minutes: number) {
  const rounded = Math.max(1, Math.round(minutes));
  if (rounded < 60) return `About ${rounded}m this month`;
  const hours = rounded / 60;
  return `About ${hours >= 10 ? Math.round(hours) : hours.toFixed(1).replace(".0", "")}h this month`;
}

function CaptureToggleRow({ title, description, checked, busy, onChange, muted = false, switchLabel }: { title: string; description: string; checked: boolean; busy: boolean; onChange: (value: boolean) => void; muted?: boolean; switchLabel?: string }) {
  return <div className="flex min-h-[66px] items-center justify-between gap-5 border-b border-[#efefe9] px-[19px] py-[13px] last:border-b-0"><div className="min-w-0"><p className={`truncate text-[17px] ${muted ? "text-[#74777e]" : "text-[#1a1c20]"}`} title={title}>{title}</p>{description && <p className="mt-1 text-[14px] text-[#74777e]">{description}</p>}</div><ToggleSwitch label={switchLabel ?? `${checked ? "Stop Dystil reading" : "Allow Dystil to read"} ${title}`} checked={checked} disabled={busy} onChange={onChange} /></div>;
}

function ToggleSwitch({ label, checked, disabled, onChange }: { label: string; checked: boolean; disabled?: boolean; onChange: (value: boolean) => void }) {
  return <button type="button" role="switch" aria-label={label} aria-checked={checked} disabled={disabled} onClick={() => onChange(!checked)} className={`h-[26px] w-[46px] shrink-0 rounded-full p-[3px] outline-none transition-colors focus-visible:ring-2 focus-visible:ring-[#0f6e56] focus-visible:ring-offset-2 disabled:cursor-wait disabled:opacity-50 ${checked ? "bg-[#1d9e75]" : "bg-[#d7d8d3]"}`}><span className={`block h-[20px] w-[20px] rounded-full bg-white shadow-[0_1px_2px_rgba(0,0,0,0.18)] transition-transform ${checked ? "translate-x-[20px]" : ""}`} /></button>;
}

function SettingsLoadingRows({ count }: { count: number }) {
  return <div aria-label="Loading settings" aria-busy="true">{Array.from({ length: count }, (_, index) => <div key={index} className="flex min-h-[66px] items-center justify-between border-b border-[#efefe9] px-[19px] last:border-b-0"><span className="h-4 w-[min(46%,240px)] animate-pulse rounded bg-[#e7e7e2]" /><span className="h-[26px] w-[46px] animate-pulse rounded-full bg-[#e7e7e2]" /></div>)}</div>;
}

function SettingsInlineError({ message, onRetry }: { message: string; onRetry: () => void }) {
  return <div role="alert" className="flex items-center justify-between gap-5 px-[18px] py-[18px]"><p className="text-[15px] leading-[1.5] text-[#8b3f32]">{message}</p><button type="button" onClick={onRetry} className="shrink-0 rounded-[8px] border border-[#d9b4ae] px-[13px] py-[8px] text-[14px] font-medium text-[#8b3f32] outline-none hover:bg-[#faece7] focus-visible:ring-2 focus-visible:ring-[#8b3f32]">Try again</button></div>;
}

function SettingsPage({ title, lede, children }: { title: string; lede: React.ReactNode; children: React.ReactNode }) { return <div className="mx-auto max-w-[880px]"><h1 className="text-[29px] font-medium tracking-[-0.02em]">{title}</h1>{lede && <p className="mb-[26px] mt-[8px] max-w-[690px] text-[18px] leading-[1.6] text-[#60636b]">{lede}</p>}{children}</div>; }
function SettingsCard({ children, padded = false, accent = false }: { children: React.ReactNode; padded?: boolean; accent?: boolean }) { return <div className={`mb-[14px] overflow-hidden rounded-[12px] border bg-white ${accent ? "border-[#c9e7db]" : "border-[#e7e7e2]"} ${padded ? "p-[20px]" : ""}`}>{children}</div>; }
function SettingLabel({ children, className = "" }: { children: React.ReactNode; className?: string }) { return <p className={`mb-[10px] text-[14px] text-[#9a9da5] ${className}`}>{children}</p>; }
function ToggleRow({ title, description, initial, onChange, disabled = false }: { title: string; description: string; initial: boolean; onChange?: (value: boolean) => void; disabled?: boolean }) { const [on, setOn] = useState(initial); return <div className="flex min-h-[66px] items-center justify-between gap-5 border-b border-[#efefe9] px-[19px] py-[13px] last:border-b-0"><div><p className="text-[17px]">{title}</p>{description && <p className="mt-1 text-[14px] text-[#9a9da5]">{description}</p>}</div><button type="button" disabled={disabled} aria-pressed={on} onClick={() => { const value = !on; setOn(value); onChange?.(value); }} className={`h-[26px] w-[46px] rounded-full p-[3px] ${on ? "bg-[#1d9e75]" : "bg-[#e7e7e2]"}`}><span className={`block h-[20px] w-[20px] rounded-full bg-white transition-transform ${on ? "translate-x-[20px]" : ""}`} /></button></div>; }
function Stat({ value, label, accent = false }: { value: string; label: string; accent?: boolean }) { return <div><p className={`text-[28px] font-medium ${accent ? "text-[#0f6e56]" : ""}`}>{value}</p><p className="mt-1 text-[14px] text-[#9a9da5]">{label}</p></div>; }
function SmallButton({ onClick, disabled, children }: { onClick: () => void; disabled?: boolean; children: React.ReactNode }) { return <button type="button" disabled={disabled} onClick={onClick} className="rounded-[9px] border border-[#e1e1dc] bg-white px-[14px] py-[9px] text-[15px] disabled:opacity-50">{children}</button>; }
function SimpleRow({ title, action }: { title: string; action: string }) { return <div className="flex min-h-[62px] items-center justify-between border-b border-[#efefe9] px-[19px] py-[13px] last:border-b-0"><span className="text-[17px]">{title}</span><button type="button" className="text-[15px] text-[#0f6e56]">{action}</button></div>; }

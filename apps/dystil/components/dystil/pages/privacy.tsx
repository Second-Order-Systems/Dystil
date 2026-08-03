"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { homeDir, join } from "@tauri-apps/api/path";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import { AppWindow, CalendarRange, Loader2, RotateCcw, Search, Trash2, X } from "lucide-react";
import { ChipRow, PrivacyCard, TextAction } from "../page-primitives";
import { CAPTURE_CATEGORIES } from "../capture-categories";

const DYSTIL_SOURCE_URL = "https://github.com/Second-Order-Systems/Dystil";

type DeletionMode = "today" | "source" | "dateRange" | "all";
type DeletionScope = {
  kind: DeletionMode;
  startDate?: string;
  endDate?: string;
  sourceKind?: "app" | "site";
  sourceName?: string;
};
type DeletionSource = { kind: "app" | "site"; name: string };
type DeletionPreview = {
  frameCount: number;
  eventCount: number;
  capturedDurationSeconds: number;
  screenshotCount: number;
  mediaBytes: number;
  oldestAt?: string | null;
  newestAt?: string | null;
  cloudCopyMayRemain: boolean;
};
type DeletionResult = {
  deletedFrames: number;
  deletedEvents: number;
  deletedScreenshots: number;
  forgottenEvidence: number;
  withdrawnFindings: number;
  cloudCopyMayRemain: boolean;
};
type CaptureCategory = { id: string; enabled: boolean };
type CaptureSource = { id: string; kind: "app" | "site"; name: string; activeMinutes: number; enabled: boolean };
type CaptureVisibility = { categories: CaptureCategory[]; sources: CaptureSource[]; sourcesError?: string | null };

const SOURCE_PREVIEW_LIMIT = 6;

const deletionChoices: Array<{ mode: DeletionMode; label: string; icon: typeof Trash2; destructive?: boolean }> = [
  { mode: "today", label: "Today so far", icon: Trash2 },
  { mode: "source", label: "One app or site", icon: AppWindow },
  { mode: "dateRange", label: "A date range", icon: CalendarRange },
  { mode: "all", label: "Everything, and start over", icon: RotateCcw, destructive: true },
];

function localDateInput(date: Date) {
  const offset = date.getTimezoneOffset() * 60_000;
  return new Date(date.getTime() - offset).toISOString().slice(0, 10);
}

function capturedDurationLabel(seconds: number, itemCount: number) {
  if (itemCount === 0) return "No captured data";
  if (seconds < 60) return "Less than a minute of data";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `About ${minutes} ${minutes === 1 ? "minute" : "minutes"} of data`;
  const hours = minutes / 60;
  const display = hours < 10 ? hours.toFixed(1).replace(".0", "") : Math.round(hours).toString();
  return `About ${display} ${display === "1" ? "hour" : "hours"} of data`;
}

function DeletionControls({ onDeleted }: { onDeleted: (message: string) => void }) {
  const today = localDateInput(new Date());
  const weekAgo = new Date();
  weekAgo.setDate(weekAgo.getDate() - 6);
  const [mode, setMode] = useState<DeletionMode | null>(null);
  const [startDate, setStartDate] = useState(localDateInput(weekAgo));
  const [endDate, setEndDate] = useState(today);
  const [sources, setSources] = useState<DeletionSource[]>([]);
  const [sourceQuery, setSourceQuery] = useState("");
  const [selectedSource, setSelectedSource] = useState<DeletionSource | null>(null);
  const [preview, setPreview] = useState<DeletionPreview | null>(null);
  const [pending, setPending] = useState<"sources" | "preview" | "delete" | null>(null);
  const [error, setError] = useState("");
  const [confirmation, setConfirmation] = useState("");

  const scope = useMemo<DeletionScope | null>(() => {
    if (!mode) return null;
    if (mode === "dateRange") return { kind: mode, startDate, endDate };
    if (mode === "source") return selectedSource
      ? { kind: mode, sourceKind: selectedSource.kind, sourceName: selectedSource.name }
      : null;
    return { kind: mode };
  }, [endDate, mode, selectedSource, startDate]);

  const loadPreview = useCallback(async (nextScope: DeletionScope) => {
    setPending("preview");
    setError("");
    setPreview(null);
    try {
      setPreview(await invoke<DeletionPreview>("preview_capture_deletion", { scope: nextScope }));
    } catch (reason) {
      setError(typeof reason === "string" ? reason : "Dystil could not check what would be deleted.");
    } finally {
      setPending(null);
    }
  }, []);

  const chooseMode = async (nextMode: DeletionMode) => {
    setMode(nextMode);
    setPreview(null);
    setError("");
    setConfirmation("");
    setSelectedSource(null);
    if (nextMode === "source" && sources.length === 0) {
      setPending("sources");
      try {
        setSources(await invoke<DeletionSource[]>("get_deletion_sources"));
      } catch (reason) {
        setError(typeof reason === "string" ? reason : "Dystil could not load your apps and sites.");
      } finally {
        setPending(null);
      }
    } else if (nextMode === "today" || nextMode === "all") {
      await loadPreview({ kind: nextMode });
    }
  };

  const selectSource = async (source: DeletionSource) => {
    setSelectedSource(source);
    await loadPreview({ kind: "source", sourceKind: source.kind, sourceName: source.name });
  };

  const deleteData = async () => {
    if (!scope || !preview) return;
    setPending("delete");
    setError("");
    try {
      const result = await invoke<DeletionResult>("delete_capture_data", { scope });
      const items = result.deletedFrames + result.deletedEvents;
      const message = mode === "all"
        ? "Dystil has deleted its work history and started fresh."
        : `Deleted ${items.toLocaleString()} captured ${items === 1 ? "item" : "items"}.`;
      setMode(null);
      setPreview(null);
      setConfirmation("");
      setSources([]);
      setSourceQuery("");
      setSelectedSource(null);
      onDeleted(message);
    } catch (reason) {
      setError(typeof reason === "string" ? reason : "Deletion did not finish. You can safely try again.");
    } finally {
      setPending(null);
    }
  };

  const filteredSources = useMemo(() => {
    const query = sourceQuery.trim().toLowerCase();
    return (query ? sources.filter((source) => source.name.toLowerCase().includes(query)) : sources).slice(0, 80);
  }, [sourceQuery, sources]);
  const itemCount = preview ? preview.frameCount + preview.eventCount : 0;
  const canDelete = Boolean(preview && (mode === "all" || itemCount > 0) && (mode !== "all" || confirmation === "DELETE"));

  return <>
    <div className="mt-[15px] flex flex-wrap gap-[9px]">
      {deletionChoices.map(({ mode: choiceMode, label, icon: Icon, destructive }) => <button
        key={choiceMode}
        type="button"
        aria-pressed={mode === choiceMode}
        onClick={() => void chooseMode(choiceMode)}
        disabled={pending === "delete"}
        className={`inline-flex items-center gap-2 rounded-[9px] border px-[14px] py-[9px] text-[14px] font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-offset-2 disabled:opacity-50 ${mode === choiceMode ? destructive ? "border-[#c67567] bg-[#fff5f2] text-[#8b3f32] focus-visible:ring-[#a95547]" : "border-[#87bfae] bg-[#f1f8f5] text-[#0f6e56] focus-visible:ring-[#1d9e75]" : destructive ? "border-[#dfbbb4] text-[#9a4a3c] hover:bg-[#fff7f5] focus-visible:ring-[#a95547]" : "border-[#deded8] text-[#30333a] hover:border-[#c7c7c0] hover:bg-[#fafaf8] focus-visible:ring-[#1d9e75]"}`}
      ><Icon aria-hidden="true" className="h-4 w-4" />{label}</button>)}
    </div>

    {mode && <div className={`mt-[16px] rounded-[11px] border p-[16px] ${mode === "all" ? "border-[#e6c4bd] bg-[#fffaf8]" : "border-[#dfe7e3] bg-[#f8faf9]"}`}>
      <div className="flex items-start justify-between gap-4">
        <div>
          <h3 className={`text-[16px] font-medium ${mode === "all" ? "text-[#7d3028]" : "text-[#1a1c20]"}`}>{deletionChoices.find((choice) => choice.mode === mode)?.label}</h3>
          <p className="mt-1 max-w-[710px] text-[14px] leading-[1.55] text-[#686c73]">
            {mode === "today" && "Deletes capture from local midnight up to now. New work will continue to be captured."}
            {mode === "source" && "Choose an app to remove everything captured in it, or a site to remove that exact domain."}
            {mode === "dateRange" && "Both dates are included. Capture outside this range stays untouched."}
            {mode === "all" && "Deletes everything Dystil has read and everything it has worked out from it. Your settings, connections, automations, and downloaded models stay."}
          </p>
        </div>
        <button type="button" aria-label="Close deletion controls" onClick={() => setMode(null)} disabled={pending === "delete"} className="rounded-md p-1 text-[#777b82] outline-none hover:bg-black/5 focus-visible:ring-2 focus-visible:ring-[#1d9e75]"><X aria-hidden="true" className="h-[18px] w-[18px]" /></button>
      </div>

      {mode === "source" && <div className="mt-[14px]">
        <label htmlFor="deletion-source-search" className="text-[13px] font-medium text-[#555a62]">Find an app or site</label>
        <div className="relative mt-1.5"><Search aria-hidden="true" className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[#92969e]" /><input id="deletion-source-search" value={sourceQuery} onChange={(event) => setSourceQuery(event.target.value)} placeholder="Search captured sources" className="h-10 w-full rounded-[8px] border border-[#d8d9d5] bg-white pl-9 pr-3 text-[14px] text-[#24272d] outline-none placeholder:text-[#9a9da5] focus:border-[#79b7a5] focus:ring-2 focus:ring-[#d9eee7]" /></div>
        {pending === "sources" ? <p className="mt-3 flex items-center gap-2 text-[14px] text-[#686c73]"><Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />Loading sources…</p> : <div className="mt-2 max-h-[184px] overflow-y-auto rounded-[8px] border border-[#e1e2de] bg-white p-1" role="listbox" aria-label="Captured apps and sites">
          {filteredSources.map((source) => <button key={`${source.kind}:${source.name}`} type="button" role="option" aria-selected={selectedSource?.kind === source.kind && selectedSource.name === source.name} onClick={() => void selectSource(source)} className={`flex w-full items-center justify-between rounded-[6px] px-3 py-2 text-left text-[14px] outline-none hover:bg-[#f3f7f5] focus-visible:ring-2 focus-visible:ring-[#1d9e75] ${selectedSource?.kind === source.kind && selectedSource.name === source.name ? "bg-[#edf7f3] text-[#0f6e56]" : "text-[#30333a]"}`}><span className="truncate">{source.name}</span><span className="ml-3 text-[12px] capitalize text-[#92969e]">{source.kind}</span></button>)}
          {filteredSources.length === 0 && <p className="px-3 py-4 text-center text-[14px] text-[#777b82]">No matching captured source.</p>}
        </div>}
      </div>}

      {mode === "dateRange" && <div className="mt-[14px] flex flex-wrap items-end gap-3">
        <label className="text-[13px] font-medium text-[#555a62]">From<input aria-label="Start date" type="date" max={endDate} value={startDate} onChange={(event) => { setStartDate(event.target.value); setPreview(null); }} className="mt-1.5 block h-10 rounded-[8px] border border-[#d8d9d5] bg-white px-3 text-[14px] font-normal text-[#24272d] outline-none focus:border-[#79b7a5] focus:ring-2 focus:ring-[#d9eee7]" /></label>
        <label className="text-[13px] font-medium text-[#555a62]">Through<input aria-label="End date" type="date" min={startDate} max={today} value={endDate} onChange={(event) => { setEndDate(event.target.value); setPreview(null); }} className="mt-1.5 block h-10 rounded-[8px] border border-[#d8d9d5] bg-white px-3 text-[14px] font-normal text-[#24272d] outline-none focus:border-[#79b7a5] focus:ring-2 focus:ring-[#d9eee7]" /></label>
        <button type="button" disabled={!startDate || !endDate || pending !== null} onClick={() => void loadPreview({ kind: "dateRange", startDate, endDate })} className="h-10 rounded-[8px] bg-[#e8f3ef] px-4 text-[14px] font-medium text-[#0f6e56] outline-none hover:bg-[#dceee8] focus-visible:ring-2 focus-visible:ring-[#1d9e75] disabled:opacity-50">Check what will be deleted</button>
      </div>}

      {pending === "preview" && <p className="mt-[14px] flex items-center gap-2 text-[14px] text-[#686c73]"><Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />Checking this history…</p>}
      {error && <p role="alert" className="mt-[14px] rounded-[8px] bg-[#fff0ec] px-3 py-2.5 text-[14px] leading-[1.5] text-[#7d3028]">{error}</p>}
      {preview && <div className="mt-[14px] border-t border-[#e1e5e2] pt-[14px]">
        <p className="text-[16px] font-medium text-[#24272d]">{capturedDurationLabel(preview.capturedDurationSeconds, itemCount)}</p>
        {itemCount === 0 && mode !== "all" && <p className="mt-3 text-[14px] text-[#686c73]">There is no captured history in this selection.</p>}
        {itemCount > 0 && <p className="mt-3 text-[13px] leading-[1.5] text-[#777b82]">{mode === "all" ? "Dystil will start again with no memory of your work." : "Anything Dystil worked out from this history will be removed too."} This cannot be undone.</p>}
        {preview.cloudCopyMayRemain && <p className="mt-3 rounded-[8px] bg-[#fff4dd] px-3 py-2 text-[13px] leading-[1.5] text-[#795b18]">Cloud sync has been used on this device. This removes the local copy; a previously synced cloud copy may remain.</p>}
        {mode === "all" && <label className="mt-4 block text-[13px] font-medium text-[#7d3028]">Type DELETE to confirm<input aria-label="Type DELETE to confirm" value={confirmation} onChange={(event) => setConfirmation(event.target.value)} autoComplete="off" className="mt-1.5 block h-10 w-full max-w-[300px] rounded-[8px] border border-[#d8aaa1] bg-white px-3 text-[14px] font-normal text-[#24272d] outline-none focus:border-[#a95547] focus:ring-2 focus:ring-[#f4d9d3]" /></label>}
        <div className="mt-4 flex flex-wrap items-center gap-3">
          <button type="button" disabled={!canDelete || pending !== null} onClick={() => void deleteData()} className={`inline-flex h-10 items-center gap-2 rounded-[8px] px-4 text-[14px] font-medium text-white outline-none focus-visible:ring-2 focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-45 ${mode === "all" ? "bg-[#a84e40] hover:bg-[#913e32] focus-visible:ring-[#a95547]" : "bg-[#176f59] hover:bg-[#0f5b48] focus-visible:ring-[#1d9e75]"}`}>{pending === "delete" && <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />}{mode === "all" ? "Delete everything and start over" : "Delete this history"}</button>
          <button type="button" disabled={pending === "delete"} onClick={() => setMode(null)} className="h-10 rounded-[8px] px-3 text-[14px] text-[#686c73] outline-none hover:text-[#24272d] focus-visible:ring-2 focus-visible:ring-[#1d9e75]">Cancel</button>
        </div>
      </div>}
    </div>}
  </>;
}

export function Privacy({ onOpenSettings }: { onOpenSettings: () => void }) {
  const [notice, setNotice] = useState("");
  const [visibility, setVisibility] = useState<CaptureVisibility | null>(null);
  const [visibilityError, setVisibilityError] = useState(false);
  const showNotice = (message: string) => { setNotice(message); window.setTimeout(() => setNotice(""), 2600); };
  const openDystilFolder = async () => {
    try {
      await openPath(await join(await homeDir(), ".dystil"));
    } catch (error) {
      console.error("Failed to open Dystil data folder", error);
      showNotice("Dystil could not open ~/.dystil.");
    }
  };
  const openSourceCode = async () => {
    try {
      await openUrl(DYSTIL_SOURCE_URL);
    } catch (error) {
      console.error("Failed to open Dystil source code", error);
      showNotice("Dystil could not open the source repository.");
    }
  };
  const loadVisibility = useCallback(async () => {
    setVisibilityError(false);
    try {
      setVisibility(await invoke<CaptureVisibility>("get_capture_visibility"));
    } catch {
      setVisibilityError(true);
    }
  }, []);
  useEffect(() => { void loadVisibility(); }, [loadVisibility]);

  const categoryChips = useMemo(() => CAPTURE_CATEGORIES.map((definition) => {
    const enabled = visibility?.categories.find((category) => category.id === definition.id)?.enabled ?? false;
    return { label: `${definition.title}${enabled ? ", on" : ""}`, enabled };
  }), [visibility?.categories]);
  const enabledCount = categoryChips.filter((category) => category.enabled).length;
  const categorySummary = enabledCount === 0
    ? "These are off. Turn on only what counts as work for you."
    : `Personal for most people, work for some. You have turned ${enabledCount === 1 ? "one" : enabledCount} on.`;
  const allSources = visibility?.sources ?? [];
  const enabledSources = allSources.filter((source) => source.enabled);
  const disabledSources = allSources.filter((source) => !source.enabled);
  const sourceCount = allSources.length;
  const sourceSummary = !visibility
    ? visibilityError ? "The source list is unavailable right now." : "Loading this month’s apps and sites…"
    : visibility.sourcesError
      ? "The source list is unavailable right now."
      : sourceCount === 0
        ? "No apps or sites have been captured this month yet."
        : `${sourceCount.toLocaleString()} ${sourceCount === 1 ? "source" : "sources"} seen this month, ordered by where your captured work happened most.`;

  return <div className="mx-auto max-w-[1124px] pb-10"><h1 className="text-[29px] font-medium tracking-[-0.02em] text-[#1a1c20]">What stays on this computer</h1><p className="mt-[10px] max-w-[680px] text-[18px] leading-[1.65] text-[#60636b]">Everything Dystil has read is in one folder on this machine. Nothing has been sent anywhere, and there is no copy of it to ask for.</p><div className="mt-[18px] rounded-[12px] bg-[#f3f8f5] px-[18px] py-[14px] text-[17px] leading-[1.55] text-[#60636b]">This page is the whole picture in one place. Anything you want to change is one click away in <button type="button" className="text-[#0f6e56]" onClick={onOpenSettings}>Settings</button>.</div>
    <PrivacyCard accent title="What never leaves">Everything Dystil reads about your work, and everything it has worked out from it.<br />Where it lives: <span className="text-[#1a1c20]">~/.dystil</span><div className="mt-[14px] flex gap-5"><TextAction onClick={() => void openDystilFolder()}>Open that folder</TextAction><TextAction onClick={() => void openSourceCode()}>Read the code that does this</TextAction></div></PrivacyCard>
    <PrivacyCard title="Never read, and there is no switch for it">No job needs these, and getting them wrong would matter too much. They are skipped in the code, not in a setting.<ChipRow items={["Passwords and credentials", "Banking and payments", "Health", "Dating and faith apps", "Private browsing windows"]} /></PrivacyCard>
    <PrivacyCard title="Off unless you say otherwise" action={<TextAction onClick={onOpenSettings}>Change in Settings</TextAction>}>
      {!visibility && !visibilityError && <div aria-label="Loading sensitive categories" className="mt-[14px] flex flex-wrap gap-[8px]">{CAPTURE_CATEGORIES.map((category) => <span key={category.id} className="h-[34px] w-[142px] animate-pulse rounded-full bg-[#efefe9]" />)}</div>}
      {visibility && <>{categorySummary}<ChipRow items={categoryChips.map((category) => category.label)} activeItems={categoryChips.filter((category) => category.enabled).map((category) => category.label)} /></>}
      {visibilityError && <div role="alert" className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[#8b3f32]">Dystil could not load these settings.<TextAction onClick={() => void loadVisibility()}>Try again</TextAction></div>}
    </PrivacyCard>
    <PrivacyCard title="Delete what it has read">Preview exactly what will be removed before anything is deleted.<DeletionControls onDeleted={showNotice} /></PrivacyCard>
    <section className="mt-[28px] border-t border-[#e7e7e2] pt-[24px]">
      <div className="flex items-start justify-between gap-6"><div><h2 className="text-[22px] font-medium text-[#1a1c20]">Apps and sites Dystil has come across</h2><p className="mt-[7px] max-w-[690px] text-[16px] leading-[1.6] text-[#60636b]">{sourceSummary}</p></div><TextAction onClick={onOpenSettings}>Manage in Settings</TextAction></div>
      {!visibility && !visibilityError && <div aria-label="Loading apps and sites" className="mt-[18px] flex flex-wrap gap-[8px]">{Array.from({ length: 6 }, (_, index) => <span key={index} className="h-[34px] w-[112px] animate-pulse rounded-full bg-[#efefe9]" />)}</div>}
      {visibility?.sourcesError && <div role="alert" className="mt-[14px] flex flex-wrap items-center gap-x-3 gap-y-1 text-[15px] text-[#8b3f32]">Recent apps and sites are unavailable while the capture database is offline.<TextAction onClick={() => void loadVisibility()}>Try again</TextAction></div>}
      {visibility && !visibility.sourcesError && <>
        <SourcePreview label="Read from" sources={enabledSources} onOpenSettings={onOpenSettings} />
        <SourcePreview label="Turned off" sources={disabledSources} disabled onOpenSettings={onOpenSettings} />
      </>}
    </section>{notice && <div role="status" className="fixed bottom-[28px] left-1/2 z-20 -translate-x-1/2 rounded-[9px] bg-[#1a1c20] px-[18px] py-[11px] text-[15px] text-white">{notice}</div>}</div>;
}

function SourcePreview({ label, sources, disabled = false, onOpenSettings }: { label: string; sources: CaptureSource[]; disabled?: boolean; onOpenSettings: () => void }) {
  if (sources.length === 0) return null;
  const visible = sources.slice(0, SOURCE_PREVIEW_LIMIT);
  const remaining = sources.length - visible.length;
  return <div className="mt-[18px]"><p className="text-[14px] text-[#777b82]">{label}</p><div className="mt-[9px] flex flex-wrap gap-[8px]">{visible.map((source) => <span key={source.id} className={`rounded-full border px-[13px] py-[6px] text-[14px] ${disabled ? "border-[#e5e1d7] bg-[#f4f1e9] text-[#8a8e94] line-through decoration-[#aeb1b2]" : "border-[#e1e1dc] bg-white text-[#30333a]"}`}>{source.name}</span>)}{remaining > 0 && <button type="button" aria-label={`Show ${remaining} more ${disabled ? "turned-off" : "allowed"} apps and sites in Settings`} onClick={onOpenSettings} className="rounded-full border border-[#c9e7db] bg-[#f3f8f5] px-[13px] py-[6px] text-[14px] font-medium text-[#0f6e56] outline-none hover:border-[#8fcbb6] hover:bg-[#eaf5f1] focus-visible:ring-2 focus-visible:ring-[#1d9e75] focus-visible:ring-offset-2">+{remaining} more</button>}</div></div>;
}

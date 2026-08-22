"use client";

import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { commands, type AppPolicy, type AppPolicySnapshot } from "@/lib/utils/tauri";

const communityPolicy: AppPolicy = { edition: "community", localWorthFixing: "enabled", localAutomation: "enabled", localAi: "enabled", readyToUse: "enabled", askBackend: "local", capture: { availability: "enabled", permanentControl: "user", temporaryPause: "enabled", exclusionsControl: "user", localDeletion: "enabled", screenshots: "userChoice", sync: "userConsent" }, telemetryManagement: "user", updateManagement: "user", manualUpdate: "enabled", autostartManagement: "user", notifications: { delivery: "enabled", preferences: "userEditable" }, teamInvitation: "enabled" };
const browserSnapshot: AppPolicySnapshot = { status: "ready", assignment: null, policy: communityPolicy, source: null };

type AppPolicyContextState = { policy: AppPolicy | null; status: string; error: boolean; retry: () => void };
const AppPolicyContext = createContext<AppPolicyContextState>({ policy: null, status: "resolving", error: false, retry: () => {} });

export function AppPolicyProvider({ children }: { children: ReactNode }) {
  const [snapshot, setSnapshot] = useState<AppPolicySnapshot | null>(null);
  const [error, setError] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const automaticallyRetried = useRef(false);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    if (!isTauri()) { setSnapshot(browserSnapshot); return; }
    const load = async () => {
      for (let count = 0; count < 2; count += 1) {
        try {
          const next = await commands.getAppPolicySnapshot();
          if (!cancelled) { setSnapshot(next); setError(false); }
          return;
        } catch (cause) {
          const detail = cause instanceof Error ? cause.message : String(cause);
          console.error("app policy load failed", cause);
          void commands.writeBrowserLog("error", `app policy load failed: ${detail}`).catch(() => {});
        }
      }
      if (!cancelled) { void commands.recordAppPolicyLoadFailed().catch(() => {}); setError(true); }
    };
    void load();
    void listen<AppPolicySnapshot>("app-policy-changed", (event) => {
      if (!cancelled) { setSnapshot(event.payload); setError(false); }
    }).then((dispose) => { unlisten = dispose; }).catch(() => {});
    return () => { cancelled = true; unlisten?.(); };
  }, [attempt]);

  useEffect(() => {
    if (snapshot?.status !== "error") return;
    if (automaticallyRetried.current) { setError(true); return; }
    automaticallyRetried.current = true;
    void commands.authFetchProfile().catch(() => setError(true));
  }, [snapshot]);

  const retry = () => {
    automaticallyRetried.current = true;
    setError(false);
    setSnapshot(null);
    void commands.authFetchProfile().catch(() => setError(true));
    setAttempt((value) => value + 1);
  };
  if (error || snapshot?.status === "error") return <main className="grid min-h-screen place-items-center p-6"><section className="max-w-md rounded-xl border border-[#d8ddd9] bg-white p-6 text-[#1f2722]"><h1 className="text-xl font-semibold">Dystil couldn’t load its product settings</h1><p className="mt-2 text-sm leading-6 text-[#59615c]">Try again to continue.</p><button type="button" onClick={retry} className="mt-5 rounded-md bg-[#176f51] px-4 py-2 text-sm font-semibold text-white">Try again</button></section></main>;
  return <AppPolicyContext.Provider value={{ policy: snapshot?.policy ?? null, status: snapshot?.status ?? "resolving", error: false, retry }}>{children}</AppPolicyContext.Provider>;
}

export function useAppPolicy() { return useContext(AppPolicyContext); }
export { communityPolicy };

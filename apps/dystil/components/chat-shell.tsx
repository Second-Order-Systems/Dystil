"use client";

import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { usePlatform } from "@/lib/hooks/use-platform";
import { AskForFix } from "./dystil/pages/ask-for-fix";
import { Privacy } from "./dystil/pages/privacy";
import { ReadyToUse } from "./dystil/pages/ready-to-use";
import { WorthFixing } from "./dystil/pages/worth-fixing";
import { SettingsWorkspace } from "./dystil/settings-workspace";
import { Sidebar } from "./dystil/sidebar";
import type { DystilShellProps } from "./dystil/types";

export function ChatShell(props: DystilShellProps) {
  const pathname = usePathname();
  const router = useRouter();
  const searchParams = useSearchParams();
  const { isMac } = usePlatform();
  const shellClassName = `relative h-dvh min-h-[640px] min-w-[760px] overflow-hidden bg-[#fdfdfc] text-[#0d0e0d] ${isMac ? "pt-[38px]" : ""}`;
  const go = (path: string) => router.push(path);
  const settingsTab = searchParams.get("tab");
  const initialTab = settingsTab === "Invite your team" ? settingsTab : undefined;

  if (pathname.includes("/settings")) {
    return <main className={`${shellClassName} text-[#1a1c20]`}><MacDragRegion isMac={isMac} /><div className="h-full overflow-hidden bg-[#f8f8f7]"><SettingsWorkspace {...props} initialTab={initialTab} onBack={() => go("/home")} /></div></main>;
  }

  return <main className={shellClassName}><MacDragRegion isMac={isMac} /><div className="grid h-full grid-cols-[268px_minmax(0,1fr)] overflow-hidden bg-[#fdfdfc]"><Sidebar /><section className="min-h-0 overflow-y-auto px-[47px] pb-[44px] pt-[45px]">{pathname.endsWith("/ready") ? <ReadyToUse onAsk={() => go("/home/ask")} onWorthFixing={() => go("/home")} /> : pathname.endsWith("/ask") ? <AskForFix /> : pathname.endsWith("/privacy") ? <Privacy onOpenSettings={() => go("/home/settings")} /> : <WorthFixing onAsk={() => go("/home/ask")} onReady={() => go("/home/ready")} onSetup={() => go("/home/settings")} />}</section></div></main>;
}

function MacDragRegion({ isMac }: { isMac: boolean }) {
  return isMac ? <div data-tauri-drag-region className="absolute inset-x-0 top-0 h-[38px] border-b border-[#e4e2dc] bg-[#f5f3ed]" aria-hidden="true" /> : null;
}

export { Sidebar } from "./dystil/sidebar";
export type { DystilShellProps, SettingsTab } from "./dystil/types";

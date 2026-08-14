"use client";

/**
 * The application shell — a vertical flex column, full window height.
 * Spec: agent_docs/design_handoff_home_screen/README.md, "Shell".
 *
 * Replaces the old 268px sidebar grid. Order is:
 *   title bar 38px -> top bar 54px -> content (flex:1, scrolls) -> strip 34px
 */

import { usePathname, useRouter } from "next/navigation";
import { usePlatform } from "@/lib/hooks/use-platform";
import { useHome } from "@/lib/mock/provider";
import { StatusStrip } from "./status-strip";
import { TopBar } from "./top-bar";

export function AppShell({ children }: { children: React.ReactNode }) {
  const router = useRouter();
  const pathname = usePathname();
  const { queue, shortcuts, job, stopJob } = useHome();
  const { isMac } = usePlatform();
  const go = (path: string) => router.push(path);

  // The pill is a way back to the pile, so it is pointless while you are
  // already looking at it — and it is never rendered at zero.
  const onHome = pathname === "/home";

  return (
    <main className="flex h-dvh min-h-[600px] min-w-[800px] flex-col overflow-hidden bg-ground text-ink">
      {/*
        macOS traffic lights live here. This strip is also the window's drag
        region — losing `data-tauri-drag-region` makes the window undraggable,
        and no automated check catches that.
      */}
      {isMac ? (
        <div
          data-tauri-drag-region
          className="h-[38px] shrink-0 border-b border-line-2b bg-chrome"
          aria-hidden="true"
        />
      ) : null}

      <TopBar
        queueCount={queue.length}
        showQueuePill={!onHome}
        shortcutCount={shortcuts.length}
        onHome={() => go("/home")}
        onQueue={() => go("/home")}
        onInvite={() => go("/home/settings?tab=Invite your team")}
        onShortcuts={() => go("/home/ready")}
        onAsk={() => go("/home/ask")}
        onSettings={() => go("/home/settings")}
      />

      <div className="min-h-0 flex-1 overflow-y-auto">{children}</div>

      <StatusStrip
        job={job}
        onPrivacy={() => go("/home/privacy")}
        onStopJob={stopJob}
        onOpenResult={() => go("/home")}
      />
    </main>
  );
}

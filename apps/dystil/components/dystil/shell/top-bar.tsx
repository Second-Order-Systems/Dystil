"use client";

/**
 * The 54px top bar. Replaces the old 268px sidebar as the app's navigation.
 * Spec: agent_docs/design_handoff_home_screen/README.md, "Shell".
 */

import { Droplet, DystilMark } from "../primitives/droplet";

type TopBarProps = {
  /** Unsettled count. The queue pill is hidden entirely at 0. */
  queueCount: number;
  /**
   * The pill is a way back to the pile, so it is pointless while you are
   * already looking at it.
   */
  showQueuePill: boolean;
  shortcutCount: number;
  onHome: () => void;
  onQueue: () => void;
  onInvite: () => void;
  onShortcuts: () => void;
  onAsk: () => void;
  showAsk?: boolean;
  onSettings: () => void;
  teamInvitationEnabled?: boolean;
  shortcutsEnabled?: boolean;
};

export function TopBar({
  queueCount,
  showQueuePill,
  shortcutCount,
  onHome,
  onQueue,
  onInvite,
  onShortcuts,
  onAsk,
  showAsk = true,
  onSettings,
  teamInvitationEnabled = false,
  shortcutsEnabled = false,
}: TopBarProps) {
  return (
    <header className="flex h-[54px] shrink-0 items-center gap-[10px] px-[22px]">
      <button
        type="button"
        onClick={onHome}
        className="flex items-center gap-[9px] rounded-icon px-2 py-1 transition-colors hover:bg-chrome"
      >
        <DystilMark width={14} height={23} />
        <span className="text-[12.5px] font-semibold tracking-[0.19em] text-ink-2">DYSTIL</span>
      </button>

      {/* Never rendered at queueCount === 0. */}
      {showQueuePill && queueCount > 0 ? (
        <button
          type="button"
          onClick={onQueue}
          className="flex items-center gap-[6px] rounded-pill bg-marigold-tint py-[6px] pl-[10px] pr-[12px] transition-colors hover:bg-marigold-hover"
        >
          <Droplet width={8} height={11} className="text-marigold" />
          <span className="whitespace-nowrap text-[11.5px] font-bold text-marigold-text">
            {queueCount} worth fixing
          </span>
        </button>
      ) : null}

      <div className="flex-1" />

      {teamInvitationEnabled && <button
        type="button"
        onClick={onInvite}
        className="flex items-center gap-[7px] rounded-tile px-[13px] py-2 text-ui font-medium text-ink-2 transition-colors hover:bg-chrome"
      >
        <InvitePersonIcon />
        Invite your team
      </button>}

      {shortcutsEnabled && <button
        type="button"
        onClick={onShortcuts}
        className="flex items-center gap-[7px] rounded-tile px-[13px] py-2 text-ui font-medium text-ink-2 transition-colors hover:bg-chrome"
      >
        Your shortcuts
        <span className="rounded-pill bg-line-2 px-[6px] py-[1px] text-label-sm font-bold text-muted-ink-2">
          {shortcutCount}
        </span>
      </button>}

      {/* The app's one persistent action — reachable from every screen. */}
      {showAsk && <button
        type="button"
        onClick={onAsk}
        className="flex items-center gap-[7px] rounded-tile bg-green-deep px-[17px] py-[9px] text-ui font-semibold text-paper shadow-primary transition-colors hover:bg-green-deep-hover"
      >
        <Droplet width={8} height={11} className="text-droplet-on-deep" />
        Ask for a fix
      </button>}

      <button
        type="button"
        onClick={onSettings}
        aria-label="Settings"
        className="flex h-8 w-8 items-center justify-center rounded-icon transition-colors hover:bg-chrome"
      >
        <SettingsSlidersIcon />
      </button>
    </header>
  );
}

function InvitePersonIcon() {
  return (
    <svg width="14" height="12" viewBox="0 0 14 12" fill="none" aria-hidden="true">
      <circle cx="5" cy="3.4" r="2.4" stroke="hsl(var(--sage))" strokeWidth="1.3" />
      <path
        d="M1 11c0-2.2 1.8-3.6 4-3.6s4 1.4 4 3.6"
        stroke="hsl(var(--sage))"
        strokeWidth="1.3"
        strokeLinecap="round"
      />
      <path
        d="M11.6 4.2v3.4M13.3 5.9H9.9"
        stroke="hsl(var(--green-mark))"
        strokeWidth="1.3"
        strokeLinecap="round"
      />
    </svg>
  );
}

function SettingsSlidersIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
      <path
        d="M2 4.5h8M12.5 4.5H14M2 11.5h1.5M6 11.5h8"
        stroke="hsl(var(--ink-3))"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
      <circle cx="11.2" cy="4.5" r="1.7" stroke="hsl(var(--ink-3))" strokeWidth="1.5" />
      <circle cx="4.8" cy="11.5" r="1.7" stroke="hsl(var(--ink-3))" strokeWidth="1.5" />
    </svg>
  );
}

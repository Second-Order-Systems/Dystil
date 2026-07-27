
import { homeDir } from "@tauri-apps/api/path";
import { getVersion } from "@tauri-apps/api/app";
import { commands } from "@/lib/utils/tauri";
import { platform } from "@tauri-apps/plugin-os";
import { Store } from "@tauri-apps/plugin-store";
import { emit, listen } from "@tauri-apps/api/event";
import React, { createContext, useContext, useEffect, useRef, useState } from "react";
import posthog from "posthog-js";
import { User } from "../utils/tauri";
import { SettingsStore } from "../utils/tauri";
import { getAuthState, subscribeAuthState } from "@/lib/auth-session";
import { type FontSize, applyFontSize } from "@/lib/utils/font-size";
export type VadSensitivity = "low" | "medium" | "high";

export enum Shortcut {
  SHOW_DYSTIL = "show_dystil",
  START_RECORDING = "start_recording",
  STOP_RECORDING = "stop_recording",
}

export type UpdateChannel = "stable" | "beta";

// Chat history types
export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  intent?: "steer";
  turnIntentId?: string;
  timestamp: number;
  contentBlocks?: any[];

  model?: string;
  provider?: string;
  /** UI override — when set, the sidebar / panel header renders this
   *  instead of `content` for compact display (e.g. "pipe executed
   *  10:24 – 10:26" for synthetic prompts). Doesn't affect persistence
   *  or what's sent to the model. */
  displayContent?: string;
  images?: any[];
  /** Non-image attachments (PDF/DOCX/XLSX/text) extracted to text. Only
   *  metadata is stored here — the actual extracted text already lives
   *  inside `content` (folded in at send time so the model sees it).
   *  The renderer reads this to draw attachment cards above the bubble. */
  attachments?: Array<{
    name: string;
    ext: string;
    charCount: number;
    truncated: boolean;
  }>;
  interruptedBySteer?: boolean;
  steeredResponse?: boolean;
  /** Wall-clock work duration for coalesced assistant messages (pipe
   *  runs). Used by the chat renderer as a fallback when no thinking
   *  blocks contributed a duration, so the work-group can still show
   *  "Worked for X min" even when the agent emitted no thinking. */
  workDurationMs?: number;
}

export interface ChatConversation {
  id: string;
  title: string;
  messages: ChatMessage[];
  createdAt: number;
  updatedAt: number;
  /** User pinned this conversation in the chat sidebar — keeps it at the top.
   *  Persists across app restarts via the on-disk conversation file. */
  pinned?: boolean;
  /** User closed this conversation from the chat sidebar — keeps the file on
   *  disk (so deleting via close is non-destructive) but excludes it from the
   *  sidebar listing. Re-surface via a future "show hidden" UI; meanwhile a
   *  dedicated delete-forever action is the only way to actually remove. */
  hidden?: boolean;
  /** ms since epoch of the most recent USER-SENT message. Drives the
   *  sidebar sort order. Persisted so that order survives app restart;
   *  derived from messages on first hydration if not set on disk yet. */
  lastUserMessageAt?: number;
  /** Last URL the agent navigated the embedded browser sidebar to.
   *  Drives the right-side `<BrowserSidebar />` panel: when the user
   *  re-opens this conversation the panel restores to this URL.
   *  Cleared (set to undefined) when the user closes the sidebar. */
  browserState?: {
    url: string;
    updatedAt: number;
    /** User-chosen panel width in CSS pixels. Defaults to 480 if unset.
     *  Persisted so re-opening the chat restores the same layout. */
    width?: number;
    /** User has hidden the panel (still has a saved URL — a small
     *  "re-open" button is shown in the chat header). */
    collapsed?: boolean;
  };
  /** Title source priority: user > ai > fallback. Used to prevent
   *  lower-priority titles from overwriting higher-priority ones. */
  titleSource?: "user" | "ai" | "fallback";
}

export interface ChatHistoryStore {
  conversations: ChatConversation[];
  activeConversationId: string | null;
  historyEnabled: boolean;
}

// Extend SettingsStore with fields added before Rust types are regenerated
export type Settings = SettingsStore & {
  deviceId?: string;
  updateChannel?: UpdateChannel;
  chatHistory?: ChatHistoryStore;
  ignoredUrls?: string[];
  searchShortcut?: string;
  lockVaultShortcut?: string;
  /** Enable AI workflow event detection (cloud, triggers event-based pipes) */
  enableWorkflowEvents?: boolean;
  /** Filters pushed from team — merged with local filters for recording */
  teamFilters?: {
    ignoredWindows: string[];
    includedWindows: string[];
    ignoredUrls: string[];
  };
  /** Cloud archive: auto-upload and delete data older than retention period */
  cloudArchiveEnabled?: boolean;
  /** Days to keep data locally before archiving (default: 7) */
  cloudArchiveRetentionDays?: number;
  /** Sync memories (facts, preferences, decisions, insights) across devices.
   * Login-gated. */
  memoriesSyncEnabled?: boolean;
  /** Sync connected-account credentials (OAuth tokens + manual API keys)
   * across devices. Off by default and kept separate from pipes/memories on
   * purpose: it syncs secrets, so enabling it is a distinct informed choice.
   * Credentials are end-to-end encrypted in the sync blob. Login-gated. */
  connectionsSyncEnabled?: boolean;
  /** Font size for the entire app UI */
  fontSize?: FontSize;
  /** User's power mode preference — persisted so it survives app restarts */
  powerMode?: "auto" | "performance" | "battery_saver";
  /** Show restart notifications when visual capture stalls (default: false for now) */
  showRestartNotifications?: boolean;
  /** Pause all screen capture when a DRM-protected streaming app (Netflix, Disney+, etc.) or a remote-desktop client (Omnissa/VMware Horizon) is focused — they blank their windows during screen recording */
  pauseOnDrmContent?: boolean;
  /** Skip clipboard capture in the UI recorder (events + content). Defaults to true (clipboard capture OFF) — passwords / API keys often pass through the clipboard, so it's opt-in. */
  disableClipboardCapture?: boolean;
  /** Skip keyboard / typed-text capture in the UI recorder. Defaults to true (keyboard capture OFF) — the a11y tree + OCR still capture on-screen text, this only drops the raw keystroke stream where secrets get typed. */
  disableKeyboardCapture?: boolean;
  /** Auto-delete local data older than retention days (free alternative to cloud archive) */
  localRetentionEnabled?: boolean;
  /** Days to keep data locally before auto-deleting (default: 14) */
  localRetentionDays?: number;
  /** What gets deleted past the cutoff: "media" keeps DB rows (search/timeline still
   * work), only reclaims mp4/wav/jpeg files. "all" wipes everything. Default: "media". */
  localRetentionMode?: "media" | "all";
  /** Apply macOS vibrancy effect to sidebar for a translucent glass look */
  translucentSidebar?: boolean;
  /** Hide model "thinking" reasoning blocks in chat (default: true) */
  hideThinkingBlocks?: boolean;
  /** Auto-generate chat titles with the LLM after the first message.
   *  Costs one extra inference per new chat. Disable to save tokens —
   *  chats fall back to a title derived from the first message (default: true) */
  autoGenerateChatTitles?: boolean;
  /** Notification preferences — which notification sources are enabled */
  notificationPrefs?: {
    captureStalls: boolean;
    appUpdates: boolean;
    pipeNotifications: boolean;
    /** Toast when a monitor is plugged, unplugged, or switched (clamshell, dock). Default true. */
    displayChanges?: boolean;
    /** Live-note prompt when a meeting is detected. Default true. */
    meetingLiveNotes?: boolean;
    mutedPipes: string[];
  };
  /** Remote devices to monitor pipes on (LAN addresses) */
  monitorDevices?: Array<{
    address: string;
    label?: string;
  }>;
  /** Enable recording schedule — when on, recording only runs during defined time ranges */
  scheduleEnabled?: boolean;
  /** Per-day-of-week time ranges defining when recording is active */
  scheduleRules?: Array<{
    dayOfWeek: number;
    startTime: string;
    endTime: string;
    recordMode: string;
  }>;
  apiAuth?: boolean;
  apiKey?: string;
  /** Default behavior when a meeting is detected.
   * - `"ask"` (default): the existing meeting-start notification grows
   *   a "+ HD" action. Click → starts a meeting-bound session that
   *   auto-stops when the call ends.
   * - `"always"`: every detected meeting auto-starts a session.
   * - `"never"`: no auto-action; only the manual tray timer can start
   *   one.
   * Indefinite manual mode does not exist — every session is bound to
   * either a meeting or a timer, both with hard-cap safety nets. */
  hdRecordingDefault?: "ask" | "always" | "never";
  /** Capture debounce (ms) installed while an HD session is active.
   * Default 100 ≈ 10 fps. Clamped to >= 33 ms (30 fps ceiling). */
  hdRecordingIntervalMs?: number;
  /**
   * When true the backend binds the HTTP API to 0.0.0.0 instead of 127.0.0.1
   * so other devices on the LAN can reach it. api_auth is force-enabled
   * whenever this is true — the backend mirrors the guard in
   * RecordingConfig::from_settings so the two flags stay consistent even
   * if someone edits the settings file by hand.
   */
  listenOnLan?: boolean;
  encryptStore?: boolean;
  /** Global blanket permission: allow dystil to copy browser cookies
   *  into the owned browser so the agent can browse sites the user is
   *  logged into. Revocable from the owned-browser cookie menu.
   *  Undefined = not decided yet, false = disabled, true = enabled. */
  browserCookieAccessGranted?: boolean;
  /** Windows-only: when true, closing the Home window hides it to the system
   * tray (and removes it from the taskbar) instead of minimizing. The Rust
   * close handler in src-tauri/src/main.rs reads this directly. Default off. */
  minimizeToTrayOnClose?: boolean;
}

export function getEffectiveFilters(settings: Settings) {
  const team = settings.teamFilters || { ignoredWindows: [], includedWindows: [], ignoredUrls: [] };
  return {
    ignoredWindows: [...new Set([...settings.ignoredWindows, ...team.ignoredWindows])],
    includedWindows: [...new Set([...settings.includedWindows, ...team.includedWindows])],
    ignoredUrls: [...new Set([...(settings.ignoredUrls || []), ...team.ignoredUrls])],
  };
}

export const DEFAULT_PROMPT = `Rules:
- Media: use standard markdown with angle-bracket local paths, like ![description](</path/to/file.mp4>) for videos and ![description](</path/to/image.jpg>) for images
- Always wrap local file paths in angle brackets because dystil paths often contain spaces or parentheses
- Diagrams: use \`\`\`mermaid blocks for visual summaries (flowchart, gantt, mindmap, graph)
- Activity summaries: gantt charts with apps/duration
- Workflows: flowcharts showing steps taken
- Knowledge sources: graph diagrams showing where info came from (apps, times, conversations)
- Meetings: extract speakers, decisions, action items
- Stay factual, use only provided data
`;

const DEFAULT_IGNORED_WINDOWS_IN_ALL_OS = [
  "bit",
  "VPN",
  "Trash",
  "Private",
  "Incognito",
  "Wallpaper",
  "Settings",
  "Keepass",
  "Recorder",
  "vault",
  "OBS Studio",
  "dystil",
];

const DEFAULT_IGNORED_WINDOWS_PER_OS: Record<string, string[]> = {
  macos: [
    ".env",
    "Item-0",
    "App Icon Window",
    "Battery",
    "Shortcuts",
    "WiFi",
    "BentoBox",
    "Clock",
    "Dock",
    "DeepL",
    "Control Center",
  ],
  windows: ["Nvidia", "Control Panel", "System Properties"],
  linux: ["Info center", "Discover", "Parted"],
};

function makeAuthUser(
  authState: ReturnType<typeof getAuthState>
): User | null {
  if (authState.status === "signed_out") return null;
  if (!authState.session?.session_token || !authState.user) return null;

  return {
    id: authState.user.id,
    name: authState.user.name,
    email: authState.user.email,
    image: authState.user.image,
    token: authState.session.session_token,
    api_key: null,
    credits: null,
    bio: null,
    website: null,
    contact: null,
    credits_balance: null,
  } as User;
}

function authUsersEqual(
  currentUser: User | null | undefined,
  nextUser: User | null
) {
  return (
    (currentUser?.id ?? null) === (nextUser?.id ?? null) &&
    (currentUser?.token ?? null) === (nextUser?.token ?? null) &&
    (currentUser?.email ?? null) === (nextUser?.email ?? null) &&
    (currentUser?.name ?? null) === (nextUser?.name ?? null) &&
    (currentUser?.image ?? null) === (nextUser?.image ?? null)
  );
}

let DEFAULT_SETTINGS: Settings = {
  deviceId: crypto.randomUUID(),
  isLoading: false,
  userId: "",
  analyticsId: "",
  devMode: false,
  ocrEngine: "default",
  monitorIds: ["default"],
  usePiiRemoval: true,
  asyncPiiRedaction: true,
  piiBackend: "local",
  piiRedactionLabels: ["secret"],
  port: 3030,
  dataDir: "default",
  ignoredWindows: [
  ],
  includedWindows: [],
  ignoredUrls: [],
  ignoredMeetingApps: [],
  teamFilters: { ignoredWindows: [], includedWindows: [], ignoredUrls: [] },

  analyticsEnabled: true,
  useChineseMirror: false,
  languages: [],
  updateChannel: "stable",
  autoUpdate: false,
  autoUpdatePipes: true,
  autoStartEnabled: true,
  platform: "unknown",
  disabledShortcuts: [],
  user: {
    id: null,
    name: null,
    email: null,
    image: null,
    token: null,
    api_key: null,
    credits: null,
    bio: null,
    website: null,
    contact: null,
    credits_balance: null,
  },
  syncConsent: { segments: false, screenshots: false },
  showDystilShortcut: "Control+Super+S",
  startRecordingShortcut: "Super+Alt+U",
  stopRecordingShortcut: "Super+Alt+X",
  searchShortcut: "Control+Super+K",
  lockVaultShortcut: "Super+Shift+L",
  disableVision: true,
  useAllMonitors: true,
  showShortcutOverlay: false,
  chatHistory: {
    conversations: [],
    activeConversationId: null,
    historyEnabled: true,
  },
  overlayMode: "fullscreen",
  showOverlayInScreenRecording: false,
  videoQuality: "balanced",
  cloudArchiveEnabled: false,
  cloudArchiveRetentionDays: 7,
  ignoreIncognitoWindows: true,
  pauseOnDrmContent: false,
  disableClipboardCapture: true,
  disableKeyboardCapture: true,
  localRetentionEnabled: false,
  localRetentionDays: 14,
  localRetentionMode: "media",
  encryptStore: true,
  hdRecordingDefault: "ask",
  hdRecordingIntervalMs: 100,
  fontSize: "16px",
};

export function createDefaultSettingsObject(): Settings {
  try {
    const p = platform();
    DEFAULT_SETTINGS.platform = p;
    DEFAULT_SETTINGS.ignoredWindows = [...DEFAULT_IGNORED_WINDOWS_IN_ALL_OS];
    DEFAULT_SETTINGS.ignoredWindows.push(...(DEFAULT_IGNORED_WINDOWS_PER_OS[p] ?? []));
    DEFAULT_SETTINGS.ocrEngine = p === "macos" ? "apple-native" : p === "windows" ? "windows-native" : "tesseract";
    DEFAULT_SETTINGS.showDystilShortcut = p === "windows" ? "Alt+S" : "Control+Super+S";
    DEFAULT_SETTINGS.searchShortcut = p === "windows" ? "Alt+K" : "Control+Super+K";
    DEFAULT_SETTINGS.lockVaultShortcut = p === "windows" ? "Ctrl+Shift+L" : "Super+Shift+L";

    if (p === "windows") {
      DEFAULT_SETTINGS.overlayMode = "window";
    }

    if (p === "linux") {
      DEFAULT_SETTINGS.overlayMode = "window";
    }

    return DEFAULT_SETTINGS;
  } catch (e) {
    // Fallback if platform detection fails
    return DEFAULT_SETTINGS;
  }
}

// Store singleton
let _store: Promise<Store> | undefined;

export const getStore = async () => {
  if (!_store) {
    // Use homeDir to match Rust backend's get_base_dir which uses $HOME/.dystil
    const dir = await homeDir();
    _store = Store.load(`${dir}/.dystil/store.bin`, {
      autoSave: false,
      defaults: {},
    });
  }
  return _store;
};

/** Save the store and re-encrypt store.bin on disk (keychain encryption). */
export const saveAndEncrypt = async (store: Store) => {
  await store.save();
  await commands.reencryptStore().catch(() => { });
};

// Store utilities similar to Cap's implementation
function createSettingsStore() {
  const get = async (): Promise<Settings> => {
    const store = await getStore();
    const settings = await store.get<Settings>("settings");
    if (!settings) {
      return createDefaultSettingsObject();
    }

    // Migration: Ensure existing users have deviceId for free tier tracking
    let needsUpdate = false;
    if (!settings.deviceId) {
      settings.deviceId = crypto.randomUUID();
      needsUpdate = true;
    }

    // Cloud sync is always an explicit, fresh choice. Never infer it from a
    // legacy session, device token, or old cloud-related settings.
    if (!settings.syncConsent) {
      settings.syncConsent = { segments: false, screenshots: false };
      needsUpdate = true;
    }

    // Temporary one-time migration: force restart notifications off for all
    // existing users until the stall detector is more reliable. Users can
    // still manually opt back in afterward; the marker prevents re-overriding.
    if (!(settings as any).restartNotificationsDefaultedOff) {
      settings.showRestartNotifications = false;
      (settings as any).restartNotificationsDefaultedOff = true;
      needsUpdate = true;
    }

    // Migration: Add chat history for existing users
    if (!settings.chatHistory) {
      settings.chatHistory = {
        conversations: [],
        activeConversationId: null,
        historyEnabled: true,
      };
      needsUpdate = true;
    }

    // Always override platform with runtime detection — never trust persisted value.
    // Platform can be "unknown" if it was saved during SSR or before Tauri was ready.
    try {
      const detectedPlatform = platform();
      if (settings.platform !== detectedPlatform) {
        settings.platform = detectedPlatform;
        needsUpdate = true;
      }
    } catch {
      // platform() unavailable (SSR/tests) — keep existing value
    }

    // Mark pro migration as done so the old migration doesn't re-trigger
    if (!(settings as any)._proCloudMigrationDone) {
      (settings as any)._proCloudMigrationDone = true;
      needsUpdate = true;
    }

    // Save migrations if needed
    if (needsUpdate) {
      await store.set("settings", settings);
      await saveAndEncrypt(store);
    }

    return settings;
  };

  const set = async (value: Partial<Settings>) => {
    const store = await getStore();
    const current = await get();
    let newSettings = { ...current, ...value } as Settings;
    if ("user" in value) {
    }
    await store.set("settings", newSettings);
    await saveAndEncrypt(store);
  };

  const reset = async () => {
    const store = await getStore();
    await store.set("settings", createDefaultSettingsObject());
    await saveAndEncrypt(store);
  };

  const resetSetting = async <K extends keyof Settings>(key: K) => {
    const current = await get();
    const defaultValue = createDefaultSettingsObject()[key];
    await set({ [key]: defaultValue } as Partial<Settings>);
  };

  const listen = (callback: (settings: Settings) => void) => {
    return getStore().then((store) => {
      return store.onKeyChange("settings", (newValue: Settings | null | undefined) => {
        callback(newValue || createDefaultSettingsObject());
      });
    });
  };

  return {
    get,
    set,
    reset,
    resetSetting,
    listen,
  };
}

const settingsStore = createSettingsStore();

// Context for React
interface SettingsContextType {
  settings: Settings;
  updateSettings: (updates: Partial<Settings>) => Promise<void>;
  resetSettings: () => Promise<void>;
  resetSetting: <K extends keyof Settings>(key: K) => Promise<void>;
  reloadStore: () => Promise<void>;
  getDataDir: () => Promise<string>;
  isSettingsLoaded: boolean;
  loadingError: string | null;
}

const SettingsContext = createContext<SettingsContextType | undefined>(undefined);

export const SettingsProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [settings, setSettings] = useState<Settings>(createDefaultSettingsObject());
  const [isSettingsLoaded, setIsSettingsLoaded] = useState(false);
  const [loadingError, setLoadingError] = useState<string | null>(null);

  // Load settings on mount
  useEffect(() => {
    const loadSettings = async () => {
      try {
        const loadedSettings = await settingsStore.get();
        setSettings(loadedSettings);
        setIsSettingsLoaded(true);
        setLoadingError(null);

      } catch (error) {
        console.error("Failed to load settings:", error);
        setLoadingError(error instanceof Error ? error.message : "Unknown error");
        setIsSettingsLoaded(true);
      }
    };

    loadSettings();

    // Listen for changes
    const unsubscribe = settingsStore.listen((newSettings) => {
      setSettings(newSettings);
    });

    return () => {
      unsubscribe.then((unsub) => unsub());
    };
  }, []);

  const settingsRef = useRef(settings);
  settingsRef.current = settings;

  // Identify the user in PostHog.
  useEffect(() => {
    const analyticsId = typeof settings.analyticsId === "string" ? settings.analyticsId : undefined;
    if (!analyticsId) return;

    const userId = typeof settings.user?.id === "string" ? settings.user.id : undefined;
    const distinctId = userId || analyticsId;

    const baseProps = {
      email: typeof settings.user?.email === "string" ? settings.user.email : undefined,
      name: typeof settings.user?.name === "string" ? settings.user.name : undefined,
      user_id: typeof settings.user?.id === "string" ? settings.user.id : undefined,
      machine_analytics_id: analyticsId,
    };

    getVersion()
      .then((appVersion) => {
        posthog.identify(distinctId, { ...baseProps, app_version: appVersion });
      })
      .catch(() => {
        posthog.identify(distinctId, baseProps);
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings.analyticsId, settings.user?.id]);

  useEffect(() => {
    applyFontSize(settings.fontSize);
  }, [settings.fontSize]);

  const updateSettings = async (updates: Partial<Settings>) => {
    if ("user" in updates && !updates.user) {
      emit("dystil-auth-signout").catch(() => { });
    }
    await settingsStore.set(updates);
    // Settings will be updated via the listener

  };

  const resetSettings = async () => {
    await settingsStore.reset();
    // Settings will be updated via the listener
  };

  const resetSetting = async <K extends keyof Settings>(key: K) => {
    await settingsStore.resetSetting(key);
    // Settings will be updated via the listener
  };

  const reloadStore = async () => {
    const freshSettings = await settingsStore.get();
    setSettings(freshSettings);
  };

  const getDataDir = async () => {
    const homeDirPath = await homeDir();

    if (
      settings.dataDir !== "default" &&
      settings.dataDir &&
      settings.dataDir !== ""
    )
      return settings.dataDir;

    return `${homeDirPath}/.dystil`;
  };

  useEffect(() => {
    const syncAuthState = async () => {
      const authState = getAuthState();
      const nextUser = makeAuthUser(authState);
      const currentUser = settingsRef.current.user;
      const currentToken = currentUser?.token ?? null;
      const nextToken = nextUser?.token ?? null;

      if (authUsersEqual(currentUser, nextUser)) {
        return;
      }

      await updateSettings({ user: nextUser as any });

      if (currentToken !== nextToken) {
        try {
          await commands.setCloudToken(nextToken);
        } catch (error) {
          console.warn("failed to sync cloud token to sidecar:", error);
        }
      }

      // Authentication controls cloud use only. Local capture has its own
      // lifecycle and must never start or stop because an account changes.
    };

    const unsubscribe = subscribeAuthState(() => {
      void syncAuthState();
    });

    void syncAuthState();

    return () => {
      unsubscribe();
    };
  }, []);

  const value: SettingsContextType = {
    settings,
    updateSettings,
    resetSettings,
    resetSetting,
    reloadStore,
    getDataDir,
    isSettingsLoaded,
    loadingError,
  };

  return (
    <SettingsContext.Provider value={value}>
      {children}
    </SettingsContext.Provider>
  );
};

export function useSettings(): SettingsContextType {
  const context = useContext(SettingsContext);
  if (context === undefined) {
    throw new Error("useSettings must be used within a SettingsProvider");
  }
  return context;
}

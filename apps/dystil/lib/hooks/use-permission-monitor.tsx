
"use client";

import { useEffect, useRef, useState } from "react";
import { usePathname } from "next/navigation";
import { listen } from "@tauri-apps/api/event";
import { commands } from "@/lib/utils/tauri";
import posthog from "posthog-js";
import { getAuthState, subscribeAuthState, type DystilAuthState } from "@/lib/auth-store";
import { useOnboardingStatus } from "@/lib/hooks/use-onboarding-status";
import { useSettings } from "@/lib/hooks/use-settings";
import { getBuildCapabilities } from "@/lib/build-capabilities";

interface PermissionLostPayload {
  screen_recording: boolean;
  accessibility: boolean;
  browser_automation?: boolean;
}

interface PermissionNeededPayload {
  kind: "screen_recording" | "accessibility";
}

type DeferredRecoverySignal =
  | { type: "permission-lost"; payload: PermissionLostPayload }
  | { type: "permission_needed"; payload: PermissionNeededPayload };

export const SKIP_PATHS = ["/onboarding", "/permission-recovery"];

export function shouldSkipPermissionRecovery(pathname: string | null) {
  return SKIP_PATHS.some((p) => pathname?.startsWith(p));
}

export function isAuthReady(status: DystilAuthState["status"]) {
  return status === "ready";
}

export function hasCriticalPermissionLoss(
  permissions: Awaited<ReturnType<typeof commands.doPermissionsCheck>> | null,
  screenshotsEnabled: boolean,
) {
  if (!permissions) return false;
  const screenOk =
    permissions.screenRecording === "granted" || permissions.screenRecording === "notNeeded";
  const accessibilityOk =
    permissions.accessibility === "granted" || permissions.accessibility === "notNeeded";
  return !accessibilityOk || (screenshotsEnabled && !screenOk);
}

/**
 * Hook that listens for permission-lost events from the Rust backend
 * and automatically shows the permission recovery window.
 * Accessibility is always critical. Screen Recording is critical only after
 * the user has explicitly enabled screenshot capture.
 * Browser automation is optional and never triggers the recovery modal (#2510).
 */
export function usePermissionMonitor() {
  const hasShownRef = useRef(false);
  const pendingRecoveryRef = useRef<DeferredRecoverySignal | null>(null);
  const hasCheckedStartupRecoveryRef = useRef(false);
  const authStateRef = useRef<DystilAuthState>(getAuthState());
  const onboardingPhaseRef = useRef<"uninitialized" | "fetching" | "ready">("uninitialized");
  const onboardingCompletedRef = useRef(false);
  const screenshotsEnabledRef = useRef(false);
  const [authState, setAuthState] = useState<DystilAuthState>(() => getAuthState());
  const [enterpriseManaged, setEnterpriseManaged] = useState<boolean | null>(null);
  const {
    phase: onboardingPhase,
    onboarding,
    refresh: refreshOnboardingStatus,
  } = useOnboardingStatus();
  const { settings, isSettingsLoaded } = useSettings();
  const pathname = usePathname();

  const screenshotsEnabled = enterpriseManaged === true || !settings.disableVision;

  const isRecoveryEligible =
    isAuthReady(authState.status) &&
    isSettingsLoaded &&
    enterpriseManaged !== null &&
    onboardingPhase === "ready" &&
    Boolean(onboarding?.isCompleted);

  useEffect(() => {
    void getBuildCapabilities().then((capabilities) => {
      setEnterpriseManaged(capabilities.enterpriseManaged);
    });
  }, []);

  useEffect(() => {
    authStateRef.current = authState;
  }, [authState]);

  useEffect(() => {
    onboardingPhaseRef.current = onboardingPhase;
    onboardingCompletedRef.current = Boolean(onboarding?.isCompleted);
  }, [onboarding?.isCompleted, onboardingPhase]);

  useEffect(() => {
    screenshotsEnabledRef.current = screenshotsEnabled;
  }, [screenshotsEnabled]);

  useEffect(() => {
    const unsubscribe = subscribeAuthState((next) => {
      authStateRef.current = next;
      setAuthState(next);
    });
    return () => {
      unsubscribe();
    };
  }, []);

  useEffect(() => {
    if (!isAuthReady(authState.status)) return;
    void refreshOnboardingStatus();
  }, [authState.status, refreshOnboardingStatus]);

  const showRecoveryWindow = async () => {
    if (hasShownRef.current || shouldSkipPermissionRecovery(pathname)) return;

    hasShownRef.current = true;
    try {
      await commands.showWindow("PermissionRecovery");
    } catch (error) {
      console.error("Failed to show permission recovery window:", error);
    }

    setTimeout(() => {
      hasShownRef.current = false;
    }, 300000);
  };

  const processDeferredRecovery = async (signal: DeferredRecoverySignal) => {
    if (
      !isAuthReady(authStateRef.current.status) ||
      onboardingPhaseRef.current !== "ready" ||
      !onboardingCompletedRef.current
    ) {
      pendingRecoveryRef.current = signal;
      return;
    }
    if (shouldSkipPermissionRecovery(pathname)) return;

    const permissions = await commands.doPermissionsCheck(false);
    if (!hasCriticalPermissionLoss(permissions, screenshotsEnabledRef.current)) {
      pendingRecoveryRef.current = null;
      return;
    }

    pendingRecoveryRef.current = null;
    posthog.capture(signal.type, signal.payload);
    await showRecoveryWindow();
  };

  const runStartupRecoveryCheck = async () => {
    if (hasCheckedStartupRecoveryRef.current) return;
    if (!isRecoveryEligible || shouldSkipPermissionRecovery(pathname)) {
      return;
    }
    hasCheckedStartupRecoveryRef.current = true;

    let permissions: Awaited<ReturnType<typeof commands.doPermissionsCheck>> | null = null;
    for (let attempt = 0; attempt < 3; attempt += 1) {
      permissions = await commands.doPermissionsCheck(false);
      if (!hasCriticalPermissionLoss(permissions, screenshotsEnabledRef.current)) {
        return;
      }
      if (attempt < 2) {
        await new Promise((resolve) => setTimeout(resolve, 1000));
      }
    }

    await showRecoveryWindow();
  };

  useEffect(() => {
    if (typeof window === "undefined") return;

    if (shouldSkipPermissionRecovery(pathname)) return;

    const unlisten = listen<PermissionLostPayload>("permission-lost", async (event) => {
      const { screen_recording, accessibility, browser_automation } = event.payload;

      console.log("Permission lost event received:", { screen_recording, accessibility, browser_automation });

      // Browser automation is optional — never trigger the modal for it (#2510)
      if (!screen_recording && !accessibility) return;

      const signal: DeferredRecoverySignal = {
        type: "permission-lost",
        payload: {
          screen_recording,
          accessibility,
          browser_automation,
        },
      };

      if (!isAuthReady(authStateRef.current.status)) {
        pendingRecoveryRef.current = signal;
        return;
      }

      await processDeferredRecovery(signal);
    });

    // Listen for deferred restart requests from the cooldown logic in recording.rs.
    // When a restart is blocked by cooldown, the backend schedules a deferred check
    // and emits this event if the server is still dead after cooldown expires.
    const unlistenRestart = listen("request-server-restart", async () => {
      console.log("Deferred server restart requested by backend");
      try {
        await commands.startCapture();
      } catch (error) {
        console.error("Deferred server restart failed:", error);
      }
    });

    // Listen for permission_needed events emitted when capture is blocked
    // waiting for user to grant permission via onboarding.
    // This signals that the app should show the permission flow UI.
    const unlistenNeeded = listen<PermissionNeededPayload>("permission_needed", async (event) => {
      const { kind } = event.payload;
      console.log("Permission needed event received:", kind);

      const signal: DeferredRecoverySignal = {
        type: "permission_needed",
        payload: { kind },
      };

      if (!isAuthReady(authStateRef.current.status)) {
        pendingRecoveryRef.current = signal;
        return;
      }

      await processDeferredRecovery(signal);
    });

    return () => {
      unlisten.then((fn) => fn());
      unlistenRestart.then((fn) => fn());
      unlistenNeeded.then((fn) => fn());
    };
  }, [pathname]);

  useEffect(() => {
    if (!isRecoveryEligible) return;
    const pending = pendingRecoveryRef.current;
    if (!pending) return;
    void (async () => {
      await processDeferredRecovery(pending);
      if (!pendingRecoveryRef.current) {
        await runStartupRecoveryCheck();
      }
    })();
  }, [authState.status, isRecoveryEligible, pathname]);

  useEffect(() => {
    if (!isRecoveryEligible) return;
    if (pendingRecoveryRef.current) return;
    void runStartupRecoveryCheck();
  }, [authState.status, isRecoveryEligible, pathname]);
}

/**
 * Provider component that sets up the permission monitor
 */
export function PermissionMonitorProvider({ children }: { children: React.ReactNode }) {
  usePermissionMonitor();
  return <>{children}</>;
}

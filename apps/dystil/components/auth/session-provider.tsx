
"use client";

import { listen } from "@tauri-apps/api/event";
import { type ReactNode, useEffect, useState } from "react";
import { usePathname, useRouter } from "next/navigation";

import { LoginScreen } from "@/components/auth/login-screen";
import { getBuildCapabilities, type BuildCapabilities } from "@/lib/build-capabilities";
import { bootstrapAuthSession, subscribeAuthState } from "@/lib/auth-session";
import { getAuthState, type DystilAuthState } from "@/lib/auth-store";
import { useOnboardingStatus } from "@/lib/hooks/use-onboarding-status";

function LoadingState({ label }: { label: string }) {
  return (
    <div className="flex min-h-dvh items-center justify-center px-6 py-10">
      <div className="w-full max-w-sm border border-border bg-background p-5 text-sm text-muted-foreground">
        {label}
      </div>
    </div>
  );
}

export function DystilSessionProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<DystilAuthState>(() => getAuthState());
  const [capabilities, setCapabilities] = useState<BuildCapabilities | null>(null);
  const {
    phase: onboardingPhase,
    onboarding,
    refresh: refreshOnboardingStatus,
  } = useOnboardingStatus();
  const pathname = usePathname();
  const router = useRouter();
  const isOnboardingRoute = pathname === "/onboarding";

  useEffect(() => {
    void getBuildCapabilities().then(setCapabilities);
  }, []);

  useEffect(() => {
    if (!capabilities || !capabilities.cloudAvailable) return;
    const unsubscribe = subscribeAuthState(setState);
    console.log("[auth-flow][session] provider mounted");
    void bootstrapAuthSession().catch((error) => {
      console.error("[auth-flow][session] bootstrap failed on mount", error);
      setState((current) => ({
        ...current,
        status: "error",
        error: error instanceof Error ? error.message : String(error),
      }));
    });
    const unlistenPromise = listen("dystil-auth-refresh", () => {
      console.log("[auth-flow][session] received dystil-auth-refresh");
      void bootstrapAuthSession().catch((error) => {
        console.error("[auth-flow][session] bootstrap failed on refresh", error);
        setState((current) => ({
          ...current,
          status: "error",
          error: error instanceof Error ? error.message : String(error),
        }));
      });
    });
    return () => {
      console.log("[auth-flow][session] provider unmounted");
      unsubscribe();
      unlistenPromise.then((unlisten) => unlisten()).catch(() => { });
    };
  }, [capabilities]);

  useEffect(() => {
    console.log("[auth-flow][session] auth state", {
      status: state.status,
      hasSession: Boolean(state.session?.session_token),
      hasUser: Boolean(state.user?.id),
      deviceTokenPresent: state.device_token_present,
      error: state.error,
    });
  }, [state]);

  const cloudEnabled = capabilities?.cloudAvailable === true;
  const accountReady = !cloudEnabled || state.status === "ready";

  useEffect(() => {
    if (!accountReady) return;
    void refreshOnboardingStatus();
  }, [accountReady, refreshOnboardingStatus]);

  const onboardingState =
    !accountReady || onboardingPhase !== "ready"
      ? "checking"
      : onboarding?.isCompleted
        ? "complete"
        : "incomplete";

  useEffect(() => {
    if (!accountReady) return;
    if (onboardingState !== "incomplete") return;
    if (isOnboardingRoute) return;
    router.replace("/onboarding");
  }, [accountReady, isOnboardingRoute, onboardingState, router]);

  const isLoginScreenStatus = cloudEnabled && (
    state.status === "signed_out" ||
    state.status === "error" ||
    state.status === "authenticating" ||
    state.status === "awaiting_email_verification"
  );
  const isSessionLoadingStatus = cloudEnabled && (
    state.status === "session_ready" ||
    state.status === "profile_loading" ||
    state.status === "device_registering"
  );

  return (
    <>
      {capabilities === null ? (
        <div className="fixed inset-0 z-50 bg-background">
          <LoadingState label="Starting Dystil..." />
        </div>
      ) : isLoginScreenStatus ? (
        <div className="fixed inset-0 z-50 bg-background">
          <LoginScreen />
        </div>
      ) : (cloudEnabled && state.status !== "ready") || isSessionLoadingStatus ? (
        <div className="fixed inset-0 z-50 bg-background">
          <LoadingState label="Loading account..." />
        </div>
      ) : onboardingState === "checking" ? (
        <div className="fixed inset-0 z-50 bg-background">
          <LoadingState label="Preparing onboarding..." />
        </div>
      ) : onboardingState === "incomplete" && !isOnboardingRoute ? (
        <div className="fixed inset-0 z-50 bg-background">
          <LoadingState label="Opening onboarding..." />
        </div>
      ) : children}
    </>
  );
}

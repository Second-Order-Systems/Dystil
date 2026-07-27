"use client";

import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";

import { commands, type OnboardingStore } from "@/lib/utils/tauri";

type OnboardingStatusPhase = "uninitialized" | "fetching" | "ready";

type OnboardingStatusState = {
  phase: OnboardingStatusPhase;
  onboarding: OnboardingStore | null;
  error: string | null;
  refresh: () => Promise<OnboardingStore>;
  markCompleted: () => void;
};

const DEFAULT_ONBOARDING_STATE: OnboardingStore = {
  isCompleted: true,
  completedAt: null,
  currentStep: null,
};

let inFlightRefresh: Promise<OnboardingStore> | null = null;
let listenerInitialized = false;

const useOnboardingStatusStore = create<OnboardingStatusState>((set) => ({
  phase: "uninitialized",
  onboarding: null,
  error: null,
  refresh: async () => {
    if (inFlightRefresh) {
      return inFlightRefresh;
    }

    set((current) => ({
      phase: current.onboarding ? "ready" : "fetching",
      onboarding: current.onboarding,
      error: null,
    }));

    inFlightRefresh = (async () => {
      try {
        const result = await commands.getOnboardingStatus();
        if (result.status !== "ok") {
          throw new Error(
            typeof result.error === "string"
              ? result.error
              : "Failed to read onboarding status.",
          );
        }

        set({
          phase: "ready",
          onboarding: result.data,
          error: null,
        });

        return result.data;
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        console.error("Failed to read onboarding status:", error);
        set({
          phase: "ready",
          onboarding: DEFAULT_ONBOARDING_STATE,
          error: message,
        });
        return DEFAULT_ONBOARDING_STATE;
      } finally {
        inFlightRefresh = null;
      }
    })();

    return inFlightRefresh;
  },
  markCompleted: () =>
    set((current) => ({
      phase: "ready",
      onboarding: {
        isCompleted: true,
        completedAt:
          current.onboarding?.completedAt ?? new Date().toISOString(),
        currentStep: null,
      },
      error: current.error,
    })),
}));

function ensureOnboardingCompletedListener() {
  if (listenerInitialized || typeof window === "undefined") return;
  listenerInitialized = true;

  void listen("onboarding-completed", () => {
    useOnboardingStatusStore.getState().markCompleted();
  }).catch((error) => {
    listenerInitialized = false;
    console.error("Failed to subscribe to onboarding-completed:", error);
  });
}

export function useOnboardingStatus() {
  const store = useOnboardingStatusStore();

  useEffect(() => {
    ensureOnboardingCompletedListener();
  }, []);

  return store;
}

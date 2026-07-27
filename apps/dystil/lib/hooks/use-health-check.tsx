"use client";

import { useEffect, useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands } from "@/lib/utils/tauri";

export interface HealthCheckResponse {
  status: string;
  status_code: number;
  last_frame_timestamp: string | null;
  last_ui_timestamp: string | null;
  frame_status: string;
  ui_status: string;
  message: string;
}

interface HealthSnapshot {
  health: HealthCheckResponse | null;
  isServerDown: boolean;
  isLoading: boolean;
}

const DOWN_GRACE_MS = 5_000;
const POLL_MS = 10_000;

let snapshot: HealthSnapshot = { health: null, isServerDown: false, isLoading: true };
let timer: ReturnType<typeof setInterval> | null = null;
let downTimer: ReturnType<typeof setTimeout> | null = null;
let consumers = 0;
const listeners = new Set<() => void>();

const getSnapshot = () => snapshot;
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};

function publish(next: Partial<HealthSnapshot>) {
  snapshot = { ...snapshot, ...next };
  listeners.forEach((listener) => listener());
}

function scheduleDown() {
  if (downTimer || snapshot.isServerDown) return;
  downTimer = setTimeout(() => {
    downTimer = null;
    publish({ isServerDown: true });
  }, DOWN_GRACE_MS);
}

export async function fetchHealth(): Promise<void> {
  try {
    const result = await commands.getCaptureHealth();
    if (result.status === "error") throw new Error(result.error);
    const health = result.data;
    if (downTimer) {
      clearTimeout(downTimer);
      downTimer = null;
    }
    publish({ health, isServerDown: false, isLoading: false });
  } catch (error) {
    console.warn("Dystil capture health unavailable", error);
    publish({
      health: {
        status: "error",
        status_code: 500,
        last_frame_timestamp: null,
        last_ui_timestamp: null,
        frame_status: "error",
        ui_status: "error",
        message: "Capture health unavailable",
      },
      isLoading: false,
    });
    scheduleDown();
  }
}

export function useHealthCheck() {
  const current = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  useEffect(() => {
    consumers += 1;
    if (consumers === 1) {
      void fetchHealth();
      timer = setInterval(() => void fetchHealth(), POLL_MS);
    }
    let disposed = false;
    const event = listen("capture-health-changed", () => {
      if (!disposed) void fetchHealth();
    });
    return () => {
      disposed = true;
      event.then((unlisten) => unlisten()).catch(() => {});
      consumers = Math.max(0, consumers - 1);
      if (consumers === 0 && timer) {
        clearInterval(timer);
        timer = null;
      }
    };
  }, []);

  return {
    health: current.health,
    isServerDown: current.isServerDown,
    isLoading: current.isLoading,
    fetchHealth,
    debouncedFetchHealth: fetchHealth,
  };
}

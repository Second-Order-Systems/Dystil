import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { WhenItRunsSettings } from "../settings-workspace";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  reloadStore: vi.fn().mockResolvedValue(undefined),
  getWhenItRunsSettings: vi.fn(),
  setAutostart: vi.fn(),
  pauseCaptureFor: vi.fn(),
  resumeCaptureNow: vi.fn(),
}));
const { reloadStore, getWhenItRunsSettings, setAutostart, pauseCaptureFor, resumeCaptureNow } = mocks;

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mocks.invoke(...args),
  isTauri: () => false,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("@/lib/hooks/use-settings", () => ({ useSettings: () => ({ reloadStore: mocks.reloadStore }) }));
vi.mock("@/lib/utils/permission-flow", () => ({ requestPermissionWithFlow: vi.fn() }));
vi.mock("@/lib/utils/tauri", () => ({ commands: {
  getWhenItRunsSettings: mocks.getWhenItRunsSettings,
  setAutostart: mocks.setAutostart,
  pauseCaptureFor: mocks.pauseCaptureFor,
  resumeCaptureNow: mocks.resumeCaptureNow,
  checkScreenRecordingPermission: vi.fn().mockResolvedValue("granted"),
  setScreenshotCaptureEnabled: vi.fn().mockResolvedValue({ status: "ok", data: null }),
} }));

const running = {
  autostartEnabled: true,
  screenshotEnabled: false,
  captureRunning: true,
  capturePaused: false,
  pauseUntil: null,
};

describe("When it runs settings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    reloadStore.mockResolvedValue(undefined);
    getWhenItRunsSettings.mockResolvedValue({ status: "ok", data: running });
    setAutostart.mockResolvedValue({ status: "ok", data: null });
    pauseCaptureFor.mockResolvedValue({ status: "ok", data: null });
    resumeCaptureNow.mockResolvedValue({ status: "ok", data: null });
  });

  it("renders backend state and persists autostart instead of using a UI default", async () => {
    render(<WhenItRunsSettings />);
    const autostart = await screen.findByRole("switch", { name: "Stop Dystil when you log in" });
    expect(autostart).toHaveAttribute("aria-checked", "true");
    fireEvent.click(autostart);
    await waitFor(() => expect(setAutostart).toHaveBeenCalledWith(false));
  });

  it("sends distinct pause modes to the shared backend controller", async () => {
    const first = render(<WhenItRunsSettings />);
    fireEvent.click(await screen.findByRole("button", { name: "1 hour" }));
    await waitFor(() => expect(pauseCaptureFor).toHaveBeenCalledWith("oneHour"));
    first.unmount();

    render(<WhenItRunsSettings />);
    fireEvent.click(await screen.findByRole("button", { name: "Today" }));
    await waitFor(() => expect(pauseCaptureFor).toHaveBeenCalledWith("today"));
  });

  it("shows the persisted deadline and resumes through the backend", async () => {
    getWhenItRunsSettings.mockResolvedValue({ status: "ok", data: {
        ...running,
        captureRunning: false,
        capturePaused: true,
        pauseUntil: "2026-08-04T00:00:00+05:30",
      } });
    render(<WhenItRunsSettings />);
    fireEvent.click(await screen.findByRole("button", { name: "Resume now" }));
    await waitFor(() => expect(resumeCaptureNow).toHaveBeenCalledOnce());
  });
});

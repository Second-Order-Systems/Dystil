import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { StorageSettings } from "../settings-workspace";

const mocks = vi.hoisted(() => ({
  refetch: vi.fn(),
  openPath: vi.fn(),
  getDataDir: vi.fn(),
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...await importOriginal<typeof import("@tauri-apps/api/core")>(),
  invoke: (...args: unknown[]) => mocks.invoke(...args),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: (...args: unknown[]) => mocks.openPath(...args) }));
vi.mock("@/lib/hooks/use-settings", () => ({ useSettings: () => ({ getDataDir: mocks.getDataDir }) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("@/lib/utils/tauri", () => ({ commands: {} }));

const storageView = {
  retentionDays: 90,
  totalDataSize: "908 MB",
  totalDataBytes: 952107008,
  availableSpace: "86 GB",
  availableSpaceBytes: 92341796864,
  fixedBytes: 104857600,
  dailyHistoryBytes: 20971520,
  observedDays: 5,
  estimateIsEarly: true,
};

describe("Storage settings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("ResizeObserver", class {
      observe() {}
      unobserve() {}
      disconnect() {}
    });
    mocks.getDataDir.mockResolvedValue("/home/person/.dystil");
    mocks.openPath.mockResolvedValue(undefined);
    mocks.invoke.mockResolvedValue(storageView);
  });

  it("renders measured storage values and opens the actual data directory", async () => {
    render(<StorageSettings />);

    expect(await screen.findByText("908 MB used")).toBeInTheDocument();
    expect(screen.queryByText("Captured media")).not.toBeInTheDocument();
    expect(screen.getByText("86 GB free on this device")).toBeInTheDocument();
    expect(screen.getByRole("slider", { name: "Keep raw work history for" })).toHaveAttribute("aria-valuetext", "3 months");
    expect(screen.getByText(/Early estimate: about/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Open data folder" }));
    await waitFor(() => expect(mocks.openPath).toHaveBeenCalledWith("/home/person/.dystil"));
  });

  it("requires an explicit destructive action before shortening retention", async () => {
    render(<StorageSettings />);
    await screen.findByRole("slider", { name: "Keep raw work history for" });
    fireEvent.click(screen.getByRole("button", { name: "1 month" }));
    expect(screen.getByText("About a month of your work")).toBeInTheDocument();
    expect(screen.getByText(/permanently deletes raw history older than 1 month/i)).toBeInTheDocument();
    expect(mocks.invoke).toHaveBeenCalledTimes(1);

    mocks.invoke.mockResolvedValue({ ...storageView, retentionDays: 30 });
    fireEvent.click(screen.getByRole("button", { name: "Delete older history and apply" }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenLastCalledWith("set_retention_days", { retentionDays: 30 }));
  });
});

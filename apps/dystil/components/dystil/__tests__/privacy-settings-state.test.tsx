import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Privacy } from "../pages/privacy";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  onOpenSettings: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...await importOriginal<typeof import("@tauri-apps/api/core")>(),
  invoke: (...args: unknown[]) => mocks.invoke(...args),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: vi.fn(), openUrl: vi.fn() }));

const visibility = {
  categories: [
    { id: "personalMessaging", enabled: true },
    { id: "personalEmail", enabled: false },
    { id: "jobBoards", enabled: true },
    { id: "hrLegal", enabled: false },
    { id: "payrollSalary", enabled: false },
  ],
  sources: [
    ...Array.from({ length: 7 }, (_, index) => ({ id: `app:${index + 1}`, kind: "app", name: `Work app ${index + 1}`, activeMinutes: 100 - index, enabled: true })),
    ...Array.from({ length: 7 }, (_, index) => ({ id: `site:${index + 1}`, kind: "site", name: `blocked-${index + 1}.example`, activeMinutes: 50 - index, enabled: false })),
  ],
  sourcesError: null,
};

describe("Privacy category state", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.invoke.mockResolvedValue(visibility);
  });

  it("shows the persisted category states and links back to Settings", async () => {
    render(<Privacy onOpenSettings={mocks.onOpenSettings} />);

    expect(await screen.findByText("Personal messaging, on")).toBeInTheDocument();
    expect(screen.getByText("Job boards and CVs, on")).toBeInTheDocument();
    expect(screen.getByText("You have turned 2 on.", { exact: false })).toBeInTheDocument();
    expect(screen.getByText("Personal email")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Change in Settings" }));
    expect(mocks.onOpenSettings).toHaveBeenCalledOnce();
  });

  it("shows bounded real source groups and sends overflow to Settings", async () => {
    render(<Privacy onOpenSettings={mocks.onOpenSettings} />);

    expect(await screen.findByText("Work app 1")).toBeInTheDocument();
    expect(screen.getByText("blocked-1.example")).toBeInTheDocument();
    expect(screen.queryByText("Work app 7")).not.toBeInTheDocument();
    expect(screen.queryByText("blocked-7.example")).not.toBeInTheDocument();
    expect(screen.queryByText("The work you do")).not.toBeInTheDocument();
    expect(screen.queryByText("People your work passes through")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Show 1 more allowed apps and sites in Settings" }));
    expect(mocks.onOpenSettings).toHaveBeenCalledOnce();
  });

  it("previews and deletes today's captured history", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_capture_visibility") return {
        ...visibility,
      };
      if (command === "preview_capture_deletion") return {
        frameCount: 10,
        eventCount: 2,
        capturedDurationSeconds: 4500,
        screenshotCount: 4,
        mediaBytes: 4096,
        oldestAt: "2026-08-03T00:00:00Z",
        newestAt: "2026-08-03T08:00:00Z",
        cloudCopyMayRemain: false,
      };
      if (command === "delete_capture_data") return {
        deletedFrames: 10,
        deletedEvents: 2,
        deletedScreenshots: 4,
        forgottenEvidence: 3,
        withdrawnFindings: 1,
        cloudCopyMayRemain: false,
      };
      throw new Error(`Unexpected command: ${command}`);
    });

    render(<Privacy onOpenSettings={mocks.onOpenSettings} />);
    await screen.findByText("Personal messaging, on");

    fireEvent.click(screen.getByRole("button", { name: "Today so far" }));
    expect(await screen.findByText("About 1.3 hours of data")).toBeInTheDocument();
    expect(screen.queryByText("captured items", { exact: false })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Delete this history" }));

    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("delete_capture_data", { scope: { kind: "today" } }));
    expect(await screen.findByText("Deleted 12 captured items.")).toBeInTheDocument();
  });

  it("requires an explicit typed confirmation before starting over", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_capture_visibility") return { categories: [], sources: [], sourcesError: null };
      if (command === "preview_capture_deletion") return {
        frameCount: 1,
        eventCount: 0,
        capturedDurationSeconds: 0,
        screenshotCount: 1,
        mediaBytes: 1024,
        oldestAt: null,
        newestAt: null,
        cloudCopyMayRemain: true,
      };
      if (command === "delete_capture_data") return {
        deletedFrames: 1,
        deletedEvents: 0,
        deletedScreenshots: 1,
        forgottenEvidence: 0,
        withdrawnFindings: 0,
        cloudCopyMayRemain: true,
      };
      throw new Error(`Unexpected command: ${command}`);
    });

    render(<Privacy onOpenSettings={mocks.onOpenSettings} />);
    fireEvent.click(screen.getByRole("button", { name: "Everything, and start over" }));
    const deleteButton = await screen.findByRole("button", { name: "Delete everything and start over" });
    expect(screen.getByText("Deletes everything Dystil has read and everything it has worked out from it.", { exact: false })).toBeInTheDocument();
    expect(screen.getByText("Dystil will start again with no memory of your work.", { exact: false })).toBeInTheDocument();
    expect(screen.queryByText("screenshots", { exact: false })).not.toBeInTheDocument();
    expect(deleteButton).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Type DELETE to confirm"), { target: { value: "DELETE" } });
    expect(deleteButton).toBeEnabled();
  });
});

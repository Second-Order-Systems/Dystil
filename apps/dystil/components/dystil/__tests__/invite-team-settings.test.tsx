import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { InviteTeamSettings } from "../settings-workspace";

const mocks = vi.hoisted(() => ({
  copyTextToClipboard: vi.fn(),
  toast: vi.fn(),
}));

vi.mock("@/lib/utils/tauri", () => ({
  commands: { copyTextToClipboard: (...args: unknown[]) => mocks.copyTextToClipboard(...args) },
}));
vi.mock("@/components/ui/use-toast", () => ({ toast: (...args: unknown[]) => mocks.toast(...args) }));
vi.mock("@/lib/hooks/use-settings", () => ({ useSettings: () => ({}) }));
vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...await importOriginal<typeof import("@tauri-apps/api/core")>(),
  invoke: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: vi.fn() }));

describe("Invite team settings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.copyTextToClipboard.mockResolvedValue({ status: "ok", data: null });
  });

  it("copies the public Dystil download link", async () => {
    render(<InviteTeamSettings />);

    expect(screen.queryByText("6 hrs")).not.toBeInTheDocument();
    expect(screen.getByText("Your accounts and data are never connected.", { exact: false })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Copy Dystil link" }));

    await waitFor(() => expect(mocks.copyTextToClipboard).toHaveBeenCalledWith("https://2os.ai/download"));
    expect(await screen.findByRole("button", { name: "Link copied" })).toBeInTheDocument();
  });
});

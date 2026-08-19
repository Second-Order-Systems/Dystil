import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { YourShortcuts } from "../your-shortcuts";

const mocks = vi.hoisted(() => ({
  push: vi.fn(),
  useHome: vi.fn(),
}));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: mocks.push }),
}));

vi.mock("@/lib/home/provider", () => ({
  useHome: () => mocks.useHome(),
}));

const actions = {
  copyShortcut: vi.fn(),
  buildShortcutSkill: vi.fn(),
  installShortcutSkill: vi.fn(),
  exportShortcutSkill: vi.fn(),
};

function home(shortcuts: unknown[]) {
  return { shortcuts, ...actions };
}

describe("Your shortcuts skill bundles", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("starts a build only from the explicit Build skill action", () => {
    mocks.useHome.mockReturnValue(home([
      { id: "artifact-1", title: "Review purchase orders", kind: "runbook", meta: "Saved for later" },
    ]));
    render(<YourShortcuts />);

    fireEvent.click(screen.getByRole("button", { name: "Build skill" }));

    expect(actions.buildShortcutSkill).toHaveBeenCalledTimes(1);
    expect(actions.buildShortcutSkill).toHaveBeenCalledWith("artifact-1");
    expect(mocks.push).not.toHaveBeenCalled();
  });

  it("keeps a building or failed bundle inline", () => {
    mocks.useHome.mockReturnValue(home([
      { id: "artifact-1", title: "Review purchase orders", kind: "runbook", meta: "Saved for later", bundle: { status: "running" } },
      { id: "artifact-2", title: "Prepare purchase order", kind: "runbook", meta: "Saved for later", bundle: { status: "failed", error: "Provider unavailable" } },
    ]));
    render(<YourShortcuts />);

    expect(screen.getByRole("button", { name: "Building…" })).toBeDisabled();
    expect(screen.getByText("Building your reusable skill")).toBeInTheDocument();
    expect(screen.getByText("Preparing…")).toBeInTheDocument();
    expect(screen.getByText("Couldn’t build this skill. Try again.")).toBeInTheDocument();
    expect(screen.queryByText("Provider unavailable")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Retry build" }));
    expect(actions.buildShortcutSkill).toHaveBeenCalledWith("artifact-2");
    expect(mocks.push).not.toHaveBeenCalled();
  });

  it("explains an interrupted build and lets the user restart it", () => {
    mocks.useHome.mockReturnValue(home([
      { id: "artifact-1", title: "Review purchase orders", kind: "runbook", meta: "Saved for later", bundle: { status: "interrupted" } },
    ]));
    render(<YourShortcuts />);

    expect(screen.getByText("Dystil was closed before this skill finished. Retry build to start again.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Retry build" }));
    expect(actions.buildShortcutSkill).toHaveBeenCalledWith("artifact-1");
  });

  it("groups installation and portable handoffs in one deliberate use flow", () => {
    mocks.useHome.mockReturnValue(home([
      {
        id: "artifact-1", title: "Review purchase orders", kind: "runbook", meta: "Saved for later",
        bundle: {
          bundleId: "bundle-1", status: "ready",
          targets: [
            { target: "codex", available: true, installed: false },
            { target: "claude", available: false, installed: false },
            { target: "pi", available: true, installed: true },
          ],
        },
      },
    ]));
    render(<YourShortcuts />);

    fireEvent.click(screen.getByRole("button", { name: "Install" }));

    expect(screen.getByRole("button", { name: /Claude Desktop/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /ChatGPT/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Claude Desktop/ }));
    expect(screen.getAllByText(/Upload a skill/)).not.toHaveLength(0);
    expect(screen.getByText("CLI & more options")).toBeInTheDocument();
    expect(mocks.push).not.toHaveBeenCalled();
  });
});

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AboutSettings, SettingsWorkspace } from "../settings-workspace";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  openUrl: vi.fn(),
  onLogout: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...await importOriginal<typeof import("@tauri-apps/api/core")>(),
  invoke: (...args: unknown[]) => mocks.invoke(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: vi.fn(), openUrl: (...args: unknown[]) => mocks.openUrl(...args) }));

const renderAbout = () => render(<AboutSettings userName="Jay" userEmail="jay@example.com" onLogout={mocks.onLogout} loggingOut={false} version="0.0.4" />);

describe("About settings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.openUrl.mockResolvedValue(undefined);
  });

  it("hides the redundant manual check while automatic updates are on", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_app_update_settings") return { autoUpdate: true, updaterAvailable: true };
      if (command === "set_app_auto_update") return { autoUpdate: false, updaterAvailable: true, availableVersion: null };
      throw new Error(`Unexpected command: ${command}`);
    });
    renderAbout();

    const toggle = await screen.findByRole("switch", { name: "Update Dystil automatically" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    expect(screen.queryByRole("button", { name: "Check now" })).not.toBeInTheDocument();

    fireEvent.click(toggle);
    expect(await screen.findByText("Dystil will tell you when an update is ready.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Check now" })).not.toBeInTheDocument();
    expect(mocks.invoke).toHaveBeenCalledWith("set_app_auto_update", { enabled: false });
  });

  it("offers a discovered update without a manual check step", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_app_update_settings") return { autoUpdate: false, updaterAvailable: true, availableVersion: "0.0.5" };
      if (command === "install_app_update") return null;
      throw new Error(`Unexpected command: ${command}`);
    });
    renderAbout();

    const update = await screen.findByRole("button", { name: "Update to 0.0.5" });
    expect(screen.queryByRole("button", { name: "Check now" })).not.toBeInTheDocument();
    expect(mocks.invoke).not.toHaveBeenCalledWith("install_app_update");
    fireEvent.click(update);
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("install_app_update"));
  });

  it("opens the public source repository", async () => {
    mocks.invoke.mockResolvedValue({ autoUpdate: true, updaterAvailable: true, availableVersion: null });
    renderAbout();

    fireEvent.click(await screen.findByRole("button", { name: "Open" }));
    expect(mocks.openUrl).toHaveBeenCalledWith("https://github.com/Second-Order-Systems/Dystil");
  });

  it("shows the discovered update at the bottom of the settings sidebar", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_app_update_settings") return { autoUpdate: false, updaterAvailable: true, availableVersion: "0.0.5" };
      if (command === "install_app_update") return null;
      throw new Error(`Unexpected command: ${command}`);
    });
    render(<SettingsWorkspace
      userName="Jay"
      userEmail="jay@example.com"
      peers={[]}
      agentMessages={[]}
      sessions={[]}
      onLoadSession={vi.fn()}
      onSendLocal={vi.fn()}
      onAskPeer={vi.fn()}
      onLogout={mocks.onLogout}
      loggingOut={false}
      version="0.0.4"
      initialTab="About"
      onBack={vi.fn()}
    />);

    expect(await screen.findByText("Dystil 0.0.5 is ready")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Update now" }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("install_app_update"));
  });
});

import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppPolicyProvider, useAppPolicy } from "../app-policy";

const mocks = vi.hoisted(() => ({
  isTauri: vi.fn(),
  getAppPolicySnapshot: vi.fn(),
  authFetchProfile: vi.fn(),
  recordAppPolicyLoadFailed: vi.fn(),
  writeBrowserLog: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => mocks.isTauri() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("@/lib/utils/tauri", () => ({
  commands: {
    getAppPolicySnapshot: () => mocks.getAppPolicySnapshot(),
    authFetchProfile: () => mocks.authFetchProfile(),
    recordAppPolicyLoadFailed: () => mocks.recordAppPolicyLoadFailed(),
    writeBrowserLog: (...args: unknown[]) => mocks.writeBrowserLog(...args),
  },
}));

function PolicyName() {
  const { policy } = useAppPolicy();
  return <output>{policy?.edition ?? "loading"}</output>;
}

const enterprisePolicy = {
  edition: "enterprise",
  localWorthFixing: "disabled",
  localAutomation: "disabled",
  localAi: "disabled",
  readyToUse: "disabled",
  askBackend: "cloud",
  capture: {
    availability: "enabled",
    permanentControl: "organization",
    temporaryPause: "enabled",
    exclusionsControl: "user",
    localDeletion: "enabled",
    screenshots: "organizationEnabled",
    sync: "required",
  },
  telemetryManagement: "organization",
  updateManagement: "organization",
  manualUpdate: "enabled",
  autostartManagement: "organization",
  notifications: { delivery: "enabled", preferences: "fixed" },
  teamInvitation: "disabled",
};

describe("AppPolicyProvider", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.isTauri.mockReturnValue(true);
    mocks.recordAppPolicyLoadFailed.mockResolvedValue(undefined);
    mocks.writeBrowserLog.mockResolvedValue(undefined);
    mocks.authFetchProfile.mockResolvedValue({ status: "ok", data: null });
  });

  it("uses the Community policy in browser-only development", async () => {
    mocks.isTauri.mockReturnValue(false);
    render(<AppPolicyProvider><PolicyName /></AppPolicyProvider>);

    expect(await screen.findByText("community")).toBeInTheDocument();
    expect(mocks.getAppPolicySnapshot).not.toHaveBeenCalled();
  });

  it("uses the semantic Enterprise policy returned by Tauri", async () => {
    mocks.getAppPolicySnapshot.mockResolvedValue({ status: "ready", policy: enterprisePolicy, assignment: null, source: "fresh" });
    render(<AppPolicyProvider><PolicyName /></AppPolicyProvider>);

    expect(await screen.findByText("enterprise")).toBeInTheDocument();
  });

  it("retries once, reports one private count, and offers a manual retry", async () => {
    mocks.getAppPolicySnapshot
      .mockRejectedValueOnce(new Error("local diagnostic"))
      .mockRejectedValueOnce(new Error("local diagnostic"))
      .mockResolvedValueOnce({ status: "ready", policy: enterprisePolicy, assignment: null, source: "fresh" });
    render(<AppPolicyProvider><PolicyName /></AppPolicyProvider>);

    const retry = await screen.findByRole("button", { name: "Try again" });
    expect(screen.queryByText("local diagnostic")).not.toBeInTheDocument();
    expect(mocks.getAppPolicySnapshot).toHaveBeenCalledTimes(2);
    expect(mocks.recordAppPolicyLoadFailed).toHaveBeenCalledTimes(1);

    retry.click();
    await waitFor(() => expect(screen.getByText("enterprise")).toBeInTheDocument());
    expect(mocks.recordAppPolicyLoadFailed).toHaveBeenCalledTimes(1);
  });
});

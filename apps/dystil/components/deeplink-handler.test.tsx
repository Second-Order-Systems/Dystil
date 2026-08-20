import { cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  bootstrapAuthSession: vi.fn(),
  getAuthSessionToken: vi.fn(),
  getAuthState: vi.fn(),
  getCurrent: vi.fn(),
  invoke: vi.fn(),
  listen: vi.fn(),
  onOpenUrl: vi.fn(),
  setAuthSessionToken: vi.fn(),
  setAuthState: vi.fn(),
  tauriFetch: vi.fn(),
  toast: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-deep-link", () => ({
  getCurrent: mocks.getCurrent,
  onOpenUrl: mocks.onOpenUrl,
}));

vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn(),
  listen: mocks.listen,
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/plugin-http", () => ({ fetch: mocks.tauriFetch }));
vi.mock("@/components/ui/use-toast", () => ({
  useToast: () => ({ toast: mocks.toast }),
}));
vi.mock("@/lib/auth-session", () => ({
  bootstrapAuthSession: mocks.bootstrapAuthSession,
}));
vi.mock("@/lib/auth-store", () => ({
  getAuthSessionToken: mocks.getAuthSessionToken,
  getAuthState: mocks.getAuthState,
  setAuthSessionToken: mocks.setAuthSessionToken,
  setAuthState: mocks.setAuthState,
}));
vi.mock("@/lib/build-capabilities", () => ({
  getBuildCapabilities: vi.fn(async () => ({
    cloudAvailable: true,
    cloudBaseUrl: "https://cloud.example",
  })),
}));

import { DeeplinkHandler } from "@/components/deeplink-handler";

const callbackUrl =
  "dystil://api/auth/callback/google?code=oauth-code&state=oauth-state";

const signedOutState = {
  status: "signed_out",
  session: null,
  user: null,
  device_token_present: false,
  error: null,
  pending_verification_email: null,
};

describe("DeeplinkHandler OAuth callbacks", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    mocks.getCurrent.mockResolvedValue([callbackUrl]);
    mocks.onOpenUrl.mockResolvedValue(() => {});
    mocks.listen.mockResolvedValue(() => {});
    mocks.getAuthSessionToken.mockReturnValue(null);
    mocks.getAuthState.mockReturnValue(signedOutState);
    mocks.bootstrapAuthSession.mockResolvedValue(undefined);
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "focus_existing_window") return null;
      if (command === "auth_get_state") return signedOutState;
      if (command === "auth_store_session") return signedOutState;
      throw new Error(`Unexpected command: ${command}`);
    });
  });

  afterEach(cleanup);

  it("does not fetch the same OAuth state again after a frontend remount", async () => {
    mocks.tauriFetch.mockResolvedValue(
      new Response(null, { headers: { "set-auth-token": "session-token" } }),
    );

    const first = render(<DeeplinkHandler />);
    await waitFor(() => expect(mocks.tauriFetch).toHaveBeenCalledTimes(1));
    first.unmount();

    render(<DeeplinkHandler />);
    await waitFor(() => expect(mocks.getCurrent).toHaveBeenCalledTimes(2));

    expect(mocks.tauriFetch).toHaveBeenCalledTimes(1);
    expect(mocks.setAuthSessionToken).toHaveBeenCalledWith("session-token");
    expect(mocks.setAuthSessionToken).not.toHaveBeenCalledWith(null);
  });

  it("ignores a retained callback when the native store already has a session", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "focus_existing_window") return null;
      if (command === "auth_get_state") {
        return {
          ...signedOutState,
          status: "ready",
          session: { session_token: "existing-token", expires_at: null },
        };
      }
      throw new Error(`Unexpected command: ${command}`);
    });

    render(<DeeplinkHandler />);
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("auth_get_state"));

    expect(mocks.tauriFetch).not.toHaveBeenCalled();
    expect(mocks.setAuthSessionToken).not.toHaveBeenCalledWith(null);
    expect(mocks.setAuthState).not.toHaveBeenCalled();
  });

  it("treats Better Auth's consumed-state response as a harmless replay", async () => {
    mocks.tauriFetch.mockResolvedValue(
      new Response(
        JSON.stringify({ message: "State mismatch: verification not found" }),
        { status: 400 },
      ),
    );

    render(<DeeplinkHandler />);
    await waitFor(() => expect(mocks.tauriFetch).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("auth_get_state"));

    expect(mocks.setAuthSessionToken).not.toHaveBeenCalledWith(null);
    expect(mocks.setAuthState).not.toHaveBeenCalled();
    expect(mocks.toast).not.toHaveBeenCalledWith(
      expect.objectContaining({ title: "Sign-in failed" }),
    );
  });
});

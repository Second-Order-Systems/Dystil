import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  signUpEmail: vi.fn(),
  invoke: vi.fn(),
  setAuthSessionToken: vi.fn(),
  setAuthState: vi.fn(),
  setAwaitingEmailVerification: vi.fn(),
}));

vi.mock("@/lib/auth-client", () => ({
  getAuthClient: vi.fn(async () => ({
    signUp: { email: mocks.signUpEmail },
  })),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

vi.mock("@/lib/auth-store", () => ({
  clearAuthState: vi.fn(),
  getAuthState: vi.fn(() => ({
    status: "signed_out",
    session: null,
    user: null,
    device_token_present: false,
    error: null,
    pending_verification_email: null,
  })),
  resetToSignedOut: vi.fn(),
  setAuthSessionToken: mocks.setAuthSessionToken,
  setAuthState: mocks.setAuthState,
  setAwaitingEmailVerification: mocks.setAwaitingEmailVerification,
  subscribeAuthState: vi.fn(),
}));

import { beginEmailSignUp } from "@/lib/auth-session";

describe("beginEmailSignUp", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "auth_store_session" || command === "auth_get_state") {
        return {
          status: "session_ready",
          session: { session_token: "session-token", expires_at: null },
          user: null,
          device_token_present: false,
          error: null,
          pending_verification_email: null,
        };
      }
      if (command === "auth_fetch_profile") {
        return {
          status: "ready",
          session: { session_token: "session-token", expires_at: null },
          user: { id: "user-1", email: "person@gmail.com", name: "Person", image: null, org: null },
          device_token_present: true,
          error: null,
          pending_verification_email: null,
        };
      }
      throw new Error(`Unexpected command: ${command}`);
    });
    mocks.signUpEmail.mockImplementation(
      async (
        _input: unknown,
        options: { onSuccess: (context: { response: Response }) => Promise<void> },
      ) => {
        await options.onSuccess({
          response: new Response(null, {
            headers: { "set-auth-token": "session-token" },
          }),
        });
        return { data: { user: { id: "user-1" } }, error: null };
      },
    );
  });

  it("creates a session directly and accepts a personal email", async () => {
    const result = await beginEmailSignUp(
      "Person",
      "person@gmail.com",
      "correct-horse-battery-staple",
    );

    expect(mocks.signUpEmail).toHaveBeenCalledWith(
      expect.objectContaining({ email: "person@gmail.com" }),
      expect.objectContaining({ onSuccess: expect.any(Function) }),
    );
    expect(mocks.setAuthSessionToken).toHaveBeenCalledWith("session-token");
    expect(mocks.invoke).toHaveBeenCalledWith("auth_fetch_profile", undefined);
    expect(mocks.setAwaitingEmailVerification).not.toHaveBeenCalled();
    expect(result.status).toBe("ready");
  });
});

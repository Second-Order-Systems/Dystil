
import { getAuthClient } from "@/lib/auth-client";
import {
  clearAuthState,
  getAuthState,
  resetToSignedOut,
  setAuthSessionToken,
  setAuthState,
  setAwaitingEmailVerification,
  subscribeAuthState,
  type DystilAuthState,
} from "@/lib/auth-store";
import { invoke } from "@tauri-apps/api/core";

const AUTH_CALLBACK_URL = "dystil://auth/callback";

function invokeAuthState(command: string, args?: Record<string, unknown>) {
  return invoke<DystilAuthState>(command, args);
}

function isEmailNotVerifiedError(error: unknown): boolean {
  if (error && typeof error === "object") {
    const code = (error as { code?: unknown }).code;
    if (typeof code === "string" && code.toUpperCase().includes("EMAIL_NOT_VERIFIED")) {
      return true;
    }
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string" && /email\s+not\s+verified/i.test(message)) {
      return true;
    }
  }
  if (error instanceof Error && /email\s+not\s+verified/i.test(error.message)) {
    return true;
  }
  return false;
}

async function storeSessionFromResponse(response: Response) {
  const token = response.headers.get("set-auth-token");
  if (!token) return null;
  setAuthSessionToken(token);
  const next = await invokeAuthState("auth_store_session", { token });
  setAuthState(next);
  return next;
}

async function rehydrateFromTauri() {
  const next = await invokeAuthState("auth_get_state");
  setAuthState(next);
  setAuthSessionToken(next.session?.session_token ?? null);
  return next;
}

export async function bootstrapAuthSession() {
  const current = await rehydrateFromTauri();
  if (current.session?.session_token) {
    const refreshed = await invokeAuthState("auth_fetch_profile");
    setAuthState(refreshed);
    setAuthSessionToken(refreshed.session?.session_token ?? current.session.session_token);
    return refreshed;
  }
  return current;
}

export async function beginEmailSignIn(email: string, password: string) {
  setAuthState({
    ...getAuthState(),
    status: "authenticating",
    error: null,
  });

  try {
    const authClient = await getAuthClient();
    const result = await authClient.signIn.email(
      {
        email,
        password,
        rememberMe: true,
      },
      {
        onSuccess: async (ctx: { response: Response }) => {
          await storeSessionFromResponse(ctx.response);
        },
      },
    );

    if (result.error) {
      if (isEmailNotVerifiedError(result.error)) {
        setAwaitingEmailVerification(email);
        throw new Error(result.error.message ?? "Email not verified");
      }
      setAuthState({
        ...getAuthState(),
        status: "error",
        error: result.error.message ?? "sign-in failed",
      });
      throw new Error(result.error.message ?? "sign-in failed");
    }

    return bootstrapAuthSession();
  } catch (error) {
    if (getAuthState().status === "awaiting_email_verification") {
      throw error;
    }
    setAuthState({
      ...getAuthState(),
      status: "error",
      error: error instanceof Error ? error.message : String(error),
    });
    throw error;
  }
}

export async function beginEmailSignUp(name: string, email: string, password: string) {
  setAuthState({
    ...getAuthState(),
    status: "authenticating",
    error: null,
  });

  try {
    const authClient = await getAuthClient();
    const result = await authClient.signUp.email({
      name,
      email,
      password,
      callbackURL: AUTH_CALLBACK_URL,
    });

    if (result.error) {
      setAuthState({
        ...getAuthState(),
        status: "error",
        error: result.error.message ?? "sign-up failed",
      });
      throw new Error(result.error.message ?? "sign-up failed");
    }

    setAwaitingEmailVerification(email);
    return result.data;
  } catch (error) {
    if (getAuthState().status === "awaiting_email_verification") {
      throw error;
    }
    setAuthState({
      ...getAuthState(),
      status: "error",
      error: error instanceof Error ? error.message : String(error),
    });
    throw error;
  }
}

export async function requestEmailVerification(email: string) {
  const authClient = await getAuthClient();
  const result = await authClient.sendVerificationEmail({
    email,
    callbackURL: AUTH_CALLBACK_URL,
  });

  if (result.error) {
    throw new Error(result.error.message ?? "failed to send verification email");
  }

  return result.data;
}

export async function signOut() {
  try {
    const authClient = await getAuthClient();
    await authClient.signOut();
  } finally {
    await invokeAuthState("auth_sign_out").catch(() => null);
    clearAuthState();
  }
}

export function resetAuthToSignIn() {
  resetToSignedOut();
}

export { getAuthState, subscribeAuthState };

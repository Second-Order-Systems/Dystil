
"use client";

import { useEffect, useState } from "react";

import {
  beginEmailSignIn,
  beginEmailSignUp,
  requestEmailVerification,
  resetAuthToSignIn,
} from "@/lib/auth-session";
import { openUrl } from "@tauri-apps/plugin-opener";
import { platform as getPlatform } from "@tauri-apps/plugin-os";
import { getAuthClient } from "@/lib/auth-client";
import {
  getBuildCapabilities,
  shouldShowWorkEmailGuidance,
} from "@/lib/build-capabilities";
import { Button } from "@/components/ui/button";
import {
  getAuthState,
  setAuthState,
  subscribeAuthState,
  type DystilAuthState,
} from "@/lib/auth-store";
import { SignInForm } from "@/components/auth/sign-in-form";
import { SignUpForm } from "@/components/auth/sign-up-form";
import { Google } from "@/components/auth/google-icon";
import { useToast } from "@/components/ui/use-toast";
import { ShieldCheck } from "lucide-react";

const PERSONAL_EMAIL_DOMAINS = new Set([
  "gmail.com",
  "aol.com",
  "gmx.com",
  "googlemail.com",
  "hotmail.com",
  "icloud.com",
  "live.com",
  "mail.com",
  "me.com",
  "outlook.com",
  "proton.me",
  "protonmail.com",
  "yahoo.com",
  "yandex.com",
  "ymail.com",
]);

function isPersonalEmail(email: string) {
  const domain = email.trim().toLowerCase().split("@").pop() ?? "";
  return PERSONAL_EMAIL_DOMAINS.has(domain);
}

export function LoginScreen() {
  const { toast } = useToast();
  const [state, setState] = useState<DystilAuthState>(() => getAuthState());
  const [mode, setMode] = useState<"sign-in" | "sign-up">("sign-up");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [signUpEmailError, setSignUpEmailError] = useState<string | null>(null);
  const [workspaceBuild, setWorkspaceBuild] = useState(false);
  const [showWorkEmailGuidance, setShowWorkEmailGuidance] = useState(false);

  const [resendCountdown, setResendCountdown] = useState(0);

  useEffect(() => {
    const unsubscribe = subscribeAuthState(setState);
    return () => {
      unsubscribe();
    };
  }, []);

  useEffect(() => {
    void getBuildCapabilities().then((capabilities) => {
      setWorkspaceBuild(capabilities.authMode === "workspace");
      setShowWorkEmailGuidance(shouldShowWorkEmailGuidance(capabilities));
    });
  }, []);

  useEffect(() => {
    if (resendCountdown <= 0) return;
    const id = setTimeout(() => setResendCountdown((c) => c - 1), 1000);
    return () => clearTimeout(id);
  }, [resendCountdown]);

  useEffect(() => {
    if (state.status === "awaiting_email_verification") {
      setNotice(null);
      setResendCountdown(30);
    }
  }, [state.status, state.pending_verification_email]);

  const awaitingVerification = state.status === "awaiting_email_verification";
  const resendEmail = state.pending_verification_email ?? "";
  const authBusy = state.status === "authenticating";
  const showWorkEmailHint =
    mode === "sign-up" &&
    showWorkEmailGuidance;

  const switchToSignUp = () => {
    setMode("sign-up");
    setSignUpEmailError(null);
    setAuthState({
      ...getAuthState(),
      status: "signed_out",
      error: null,
    });
    setNotice(null);
  };

  const switchToSignIn = () => {
    setMode("sign-in");
    setSignUpEmailError(null);
    setAuthState({
      ...getAuthState(),
      status: "signed_out",
      error: null,
    });
    setNotice(null);
  };

  const handleSignIn = async (email: string, password: string) => {
    setBusy(true);
    setAuthState({
      ...getAuthState(),
      status: "authenticating",
      error: null,
    });
    try {
      await beginEmailSignIn(email.trim(), password);
    } catch {
      // error already surfaced via auth store
    } finally {
      setBusy(false);
    }
  };

  const handleSignUp = async (name: string, email: string, password: string) => {
    const normalizedEmail = email.trim();
    if (workspaceBuild && isPersonalEmail(normalizedEmail)) {
      const message = "Please use your work email address to create an account.";
      setSignUpEmailError(message);
      toast({
        title: "Work email required",
        description: message,
        variant: "destructive",
        showWhenNotificationsDisabled: true,
      });
      return;
    }

    setSignUpEmailError(null);
    setBusy(true);
    setAuthState({
      ...getAuthState(),
      status: "authenticating",
      error: null,
    });
    try {
      await beginEmailSignUp(name.trim(), normalizedEmail, password);
    } catch (error) {
      toast({
        title: "Sign-up failed",
        description: error instanceof Error ? error.message : "Please try again",
        variant: "destructive",
        showWhenNotificationsDisabled: true,
      });
    } finally {
      setBusy(false);
    }
  };

  const resendVerification = async () => {
    if (!resendEmail) return;
    setBusy(true);
    setNotice(null);
    try {
      await requestEmailVerification(resendEmail);
      setNotice("Verification email sent.");
      setResendCountdown(30);
    } catch (err) {
      setAuthState({
        ...getAuthState(),
        status: "awaiting_email_verification",
        error: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setBusy(false);
    }
  };

  const handleSocialLogin = async (provider: "google" | "github") => {
    setBusy(true);
    setAuthState({
      ...getAuthState(),
      status: "authenticating",
      error: null,
    });
    try {
      const currentPlatform = getPlatform();
      console.log("[auth-flow][login] starting social sign-in", {
        provider,
        platform: currentPlatform,
      });
      const authClient = await getAuthClient();
      const result = await authClient.signIn.social({
        provider,
        disableRedirect: true,
        fetchOptions: {
          headers: { Platform: currentPlatform },
        },
      });

      if (result.error) {
        console.error("[auth-flow][login] social sign-in failed before redirect", {
          provider,
          platform: currentPlatform,
          message: result.error.message,
        });
        throw new Error(result.error.message ?? "Social sign-in failed");
      }

      if (result.data?.url) {
        console.log("[auth-flow][login] opening provider url", {
          provider,
          platform: currentPlatform,
          hasUrl: true,
        });
        await openUrl(result.data.url);
      } else {
        console.warn("[auth-flow][login] provider did not return url", {
          provider,
          platform: currentPlatform,
        });
      }

      setAuthState({
        status: "authenticating",
        session: null,
        user: null,
        device_token_present: false,
        error: null,
        pending_verification_email: null,
      });
      // The external browser flow is now in progress. The auth-store state
      // keeps the controls disabled until the deep-link callback completes.
      setBusy(false);
    } catch (err) {
      console.error("[auth-flow][login] social sign-in errored", {
        provider,
        message: err instanceof Error ? err.message : String(err),
      });
      setBusy(false);
      setAuthState({
        status: "signed_out",
        session: null,
        user: null,
        device_token_present: false,
        error: err instanceof Error ? err.message : String(err),
        pending_verification_email: null,
      });
    }
  };

  const socialButtons = (
    <div className="grid gap-2">
      <Button
        type="button"
        variant="outline"
        className="relative h-12 w-full justify-center rounded-full px-4 py-3.5 border-[0.5px] border-white"
        disabled={busy || authBusy}
        onClick={() => handleSocialLogin("google")}
      >
        <span className="absolute left-2 top-1/2 inline-flex h-8 w-8 -translate-y-1/2 items-center justify-center rounded-full bg-white shadow-sm">
          <Google className="h-[18px] w-[18px] shrink-0" />
        </span>
        <span className="flex-1 text-center">Sign in with Google</span>
      </Button>
    </div>
  );

  return (
    <div className="h-dvh overflow-y-auto overscroll-contain bg-[radial-gradient(900px_500px_at_85%_-10%,#ebf4ef_0%,transparent_60%),#f7f8f6]">
      <div className="relative flex min-h-full w-full items-center justify-center px-6 py-10">
        <div className="relative w-full max-w-6xl">
        <div className="mx-auto flex w-full max-w-md flex-col items-center space-y-4">
          <div className="flex flex-col items-center text-center">
            <div className="flex items-center gap-[0.15em] text-4xl font-semibold tracking-[0.2em] text-foreground uppercase">
              <span>D</span><span className="text-primary">Y</span><span>STIL</span>
            </div>
            <span className="text-xs font-mono tracking-widest -ml-1">by <u>Second Order Systems</u></span>
          </div>

          {notice ? (
            <div className="border border-border bg-muted px-3 py-2 text-sm text-foreground">
              {notice}
            </div>
          ) : null}

          {awaitingVerification ? (
            <div className="space-y-4">
              <div className="border border-border bg-muted px-3 py-2 text-sm text-foreground">
                Check your inbox or spam folder for a verification link
                {resendEmail ? ` at ${resendEmail}` : ""}.
              </div>

              <Button
                type="button"
                variant="ghost"
                className="w-full"
                disabled={busy || resendCountdown > 0}
                onClick={() => void resendVerification()}
              >
                {resendCountdown > 0
                  ? `Resend verification email (${resendCountdown}s)`
                  : "Resend verification email"}
              </Button>

              <Button
                type="button"
                variant="ghost"
                className="w-full"
                disabled={busy}
                onClick={() => {
                  resetAuthToSignIn();
                  setMode("sign-in");
                  setNotice(null);
                }}
              >
                Already have an account? Sign in
              </Button>
            </div>
          ) : mode === "sign-in" ? (
            <SignInForm
              isPending={busy || authBusy}
              onSubmit={handleSignIn}
              onSignUp={switchToSignUp}
              socialButtons={socialButtons}
            />
          ) : (
            <SignUpForm
              isPending={busy || authBusy}
              onSubmit={handleSignUp}
              emailError={signUpEmailError}
              onEmailChange={() => setSignUpEmailError(null)}
              onSignIn={switchToSignIn}
              socialButtons={socialButtons}
            />
          )}
        </div>

        {showWorkEmailHint ? (
          <aside className="absolute left-[calc(50%+220px)] top-[190px] hidden w-[280px] rounded-2xl border border-[#d8e3d9] bg-[#f8fbf8] p-6 text-[#173c2b] shadow-sm lg:block">
            <div className="mb-5 flex h-14 w-14 items-center justify-center rounded-full bg-[#e4f0e5]">
              <ShieldCheck className="h-7 w-7" strokeWidth={1.8} />
            </div>
            <h2 className="text-lg font-semibold">Use your work email</h2>
            <p className="mt-4 text-sm leading-7 text-[#294839]">
              This helps us connect you with your organization.
            </p>
            <p className="mt-2 text-sm leading-7 text-[#294839]">
              If you continue with Google, choose your work account.
            </p>
            <div className="pointer-events-none absolute bottom-0 right-0 h-20 w-28 rounded-br-2xl rounded-tl-[100%] bg-[radial-gradient(ellipse_at_bottom_right,rgba(96,150,111,0.16),transparent_68%)]" />
          </aside>
        ) : null}
        </div>
      </div>
    </div>
  );
}

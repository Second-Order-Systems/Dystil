"use client";

import { useEffect } from "react";
import { useToast } from "@/components/ui/use-toast";
// import { commands } from "@/lib/utils/tauri";
import { listen, emit } from "@tauri-apps/api/event";
import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";
// import { openSettingsWindow } from "@/lib/utils/window";
// import {
//   openDystilViewerLink,
//   dystilViewerPathFromHref,
// } from "@/components/markdown";
import { bootstrapAuthSession } from "@/lib/auth-session";
import {
  getAuthSessionToken,
  getAuthState,
  setAuthSessionToken,
  setAuthState,
  type DystilAuthState,
} from "@/lib/auth-store";
import { getBuildCapabilities } from "@/lib/build-capabilities";
import {
  claimOAuthCallback,
  isConsumedOAuthStateError,
  markOAuthCallbackProcessed,
  oauthCallbackIdentity,
  releaseOAuthCallbackClaim,
} from "@/lib/oauth-callback-replay";
import { invoke } from "@tauri-apps/api/core";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";

const AUTH_BASE_PATHS = ["/api/auth/individual/", "/api/auth/"];
const GOOGLE_SIGN_IN_FAILED =
  "Google sign-in could not be completed. If you are using a personal Gmail account, please use your Google Workspace account instead.";
const activeOAuthCallbacks = new Set<string>();

export function DeeplinkHandler() {
  const { toast } = useToast();

  useEffect(() => {
    console.log("[auth-flow][deeplink] handler mounted");

    const logError = (scope: string, error: unknown) => {
      console.error(`[auth-flow][deeplink] ${scope}`, error);
    };

    const authBasePathForUrl = (url: string) =>
      AUTH_BASE_PATHS.find((path) => url.includes(path));

    const hasStoredSession = async () => {
      if (getAuthSessionToken() || getAuthState().session?.session_token) return true;
      try {
        const state = await invoke<DystilAuthState>("auth_get_state");
        return Boolean(state.session?.session_token);
      } catch (error) {
        logError("auth_get_state failed while checking callback", error);
        return false;
      }
    };

    const processDeepLinkUrl = async (url: string, source: string) => {
      const authBasePath = authBasePathForUrl(url);
      console.log("[auth-flow][deeplink] received url", {
        source,
        kind: authBasePath ? "auth" : "other",
      });
      // Deeplinks should reuse the window the user already has open.
      // Explicit navigation links below may still open their own surface,
      // but generic handoff/focus URLs should not spawn a new one.
      await invoke("focus_existing_window")
        .then(() => {
          console.log("[auth-flow][deeplink] focus_existing_window succeeded", { source });
        })
        .catch((error) => {
          logError("focus_existing_window failed", error);
        });

      // Handle Better Auth OAuth callback deep links from the tauri() plugin.
      // The plugin constructs dystil:///api/auth/callback/... but the OS
      // normalizes to dystil://api/auth/callback/... (double slash, "api" as
      // host). The plugin's own handleAuthDeepLink can't parse this format,
      // so we intercept it here as a fallback.
      const capabilities = await getBuildCapabilities();
      if (!capabilities.cloudAvailable || !capabilities.cloudBaseUrl) {
        return;
      }

      if (authBasePath && !url.includes(`${authBasePath}verify-email`)) {
        const callbackIdentity = oauthCallbackIdentity(url, authBasePath);
        if (activeOAuthCallbacks.has(callbackIdentity) || !claimOAuthCallback(callbackIdentity)) {
          console.log("[auth-flow][deeplink] ignored duplicate auth callback", { source });
          return;
        }
        activeOAuthCallbacks.add(callbackIdentity);
        let callbackResponded = false;

        console.log("[auth-flow][deeplink] auth callback matched", { source });
        try {
          if (await hasStoredSession()) {
            markOAuthCallbackProcessed(callbackIdentity);
            console.log("[auth-flow][deeplink] ignored auth callback for existing session", {
              source,
            });
            return;
          }

          const pathStart = url.indexOf(authBasePath);
          if (pathStart === -1) {
            console.warn("[auth-flow][deeplink] auth callback missing auth path", { source, url });
            return;
          }
          const baseUrl = capabilities.cloudBaseUrl;
          const href = url.slice(pathStart);
          const fullUrl = `${baseUrl}${href}`;
          console.log("[auth-flow][deeplink] fetching callback", { source, authBasePath });

          const resp = await tauriFetch(fullUrl, { method: "GET", maxRedirections: 0 } as any);
          callbackResponded = true;
          markOAuthCallbackProcessed(callbackIdentity);
          console.log("[auth-flow][deeplink] callback response", {
            source,
            status: resp.status,
            ok: resp.ok,
          });

          const token = resp.headers.get("set-auth-token");
          if (token) {
            console.log("[auth-flow][deeplink] set-auth-token present", {
              source,
              tokenLength: token.length,
            });
            setAuthSessionToken(token);
            await invoke("auth_store_session", { token });
            console.log("[auth-flow][deeplink] auth_store_session succeeded", { source });
          } else {
            console.warn("[auth-flow][deeplink] callback missing set-auth-token", {
              source,
              status: resp.status,
              location: resp.headers.get("location"),
            });
            const responseBody = await resp.clone().text().catch(() => "");
            const alreadyConsumed = isConsumedOAuthStateError(responseBody);
            const existingSession = await hasStoredSession();
            if (alreadyConsumed || existingSession) {
              if (existingSession) await bootstrapAuthSession();
              console.log("[auth-flow][deeplink] ignored non-destructive callback replay", {
                source,
                alreadyConsumed,
                existingSession,
              });
              return;
            }

            setAuthState({
              ...getAuthState(),
              status: "error",
              session: null,
              user: null,
              error: GOOGLE_SIGN_IN_FAILED,
            });
            toast({
              title: "Sign-in failed",
              description: GOOGLE_SIGN_IN_FAILED,
              variant: "destructive",
              showWhenNotificationsDisabled: true,
            });
            return;
          }

          await bootstrapAuthSession();
          console.log("[auth-flow][deeplink] bootstrapAuthSession succeeded", { source });
          toast({ title: "Signed in", description: "Your Dystil session is ready" });
        } catch (error) {
          logError("auth callback failed", error);
          toast({
            title: "Sign-in failed",
            description: "Please try again",
            variant: "destructive",
            showWhenNotificationsDisabled: true,
          });
        } finally {
          activeOAuthCallbacks.delete(callbackIdentity);
          if (!callbackResponded) releaseOAuthCallbackClaim(callbackIdentity);
        }
        return;
      }

      // Handle email verification deep link.
      // The verify-email endpoint returns 302 to /auth/callback — a client-side
      // route. The 302 means the email was verified successfully. We don't need
      // to follow the redirect or extract a token; the user signs in separately.
      if (authBasePath && url.includes(`${authBasePath}verify-email`)) {
        console.log("[auth-flow][deeplink] verify-email matched", { source });
        try {
          const pathStart = url.indexOf(authBasePath);
          if (pathStart === -1) {
            console.warn("[auth-flow][deeplink] verify-email missing auth path", { source, url });
            return;
          }
          const href = url.slice(pathStart);
          const baseUrl = capabilities.cloudBaseUrl;
          const fullUrl = `${baseUrl}${href}`;
          console.log("[auth-flow][deeplink] fetching verify-email", { source, href, fullUrl });

          const resp = await tauriFetch(fullUrl, { method: "GET", maxRedirections: 0 } as any);
          console.log("[auth-flow][deeplink] verify-email response", {
            source,
            status: resp.status,
          });

          // 302 means the token was valid and email is now verified.
          // 200 means the tauri plugin handled it directly.
          if (resp.status === 302 || resp.status === 200) {
            toast({
              title: "Email verified",
              description: "You can now sign in with your email and password",
            });
          } else {
            toast({
              title: "Verification failed",
              description: "Try opening verification email again, or request new verification link",
              variant: "destructive",
            });
          }
        } catch (error) {
          logError("verify-email failed", error);
          toast({
            title: "verification failed",
            description: error instanceof Error ? error.message : "unknown error",
            variant: "destructive",
          });
        }
        return;
      }

      // const parsedUrl = new URL(url);

      // Handle Google Calendar OAuth callback
      // if (
      //   parsedUrl.host === "auth" &&
      //   parsedUrl.pathname?.includes("google-calendar")
      // ) {
      //   console.log("[auth] >>> MATCHED google-calendar callback");
      //   const success = parsedUrl.searchParams.get("success") === "true";
      //   const error = parsedUrl.searchParams.get("error");
      //   await emit("google-calendar-auth-result", { success, error });
      //   await openSettingsWindow();
      //   toast({
      //     title: success
      //       ? "google calendar connected!"
      //       : "google calendar connection failed",
      //     description: success
      //       ? "your google calendar is now linked"
      //       : error || "something went wrong",
      //     variant: success ? undefined : "destructive",
      //   });
      // }
      //
      // if (url.includes("changelog")) {
      //   setShowChangelogDialog(true);
      // }
      //
      // if (url.includes("status")) {
      //   openStatusDialog();
      // }
      //
      // if (parsedUrl.host === "view" || parsedUrl.pathname === "view") {
      //   const filePath = dystilViewerPathFromHref(url);
      //   if (filePath) {
      //     try {
      //       await openDystilViewerLink(url);
      //     } catch (error) {
      //       console.error("Failed to open viewer:", error);
      //       toast({
      //         title: "couldn't open file",
      //         description: filePath,
      //         variant: "destructive",
      //       });
      //     }
      //   }
      // }
    };

    const setupDeepLink = async () => {
      console.log("[auth-flow][deeplink] registering onOpenUrl listener");
      const unsubscribeDeepLink = await onOpenUrl(async (urls) => {
        console.log("[auth-flow][deeplink] onOpenUrl fired", { count: urls.length });
        for (const url of urls) {
          await processDeepLinkUrl(url, "onOpenUrl");
        }
      });
      console.log("[auth-flow][deeplink] onOpenUrl listener registered");
      return unsubscribeDeepLink;
    };

    let deepLinkUnsubscribe: (() => void) | undefined;

    setupDeepLink().then((unsubscribe) => {
      deepLinkUnsubscribe = unsubscribe;
      console.log("[auth-flow][deeplink] listener ready");
    });

    void getCurrent()
      .then(async (urls) => {
        console.log("[auth-flow][deeplink] getCurrent resolved", {
          count: urls?.length ?? 0,
        });
        if (!urls?.length) return;
        for (const url of urls) {
          await processDeepLinkUrl(url, "getCurrent");
        }
      })
      .catch((error) => {
        logError("getCurrent failed", error);
      });

    const unlisten = Promise.all([
      // Listen for deep-link URLs forwarded from single-instance handoff
      // (emitted by the /focus endpoint or the single-instance plugin callback)
      listen<string>("deep-link-received", async (event) => {
        console.log("[auth-flow][deeplink] deep-link-received event", {
          url: event.payload,
        });
        await processDeepLinkUrl(event.payload, "deep-link-received");
      }),

      listen("cli-login", async (event) => {
        console.log("[auth-flow][deeplink] cli-login event", event);
        await emit("dystil-auth-refresh");
      }),
    ]);

    return () => {
      console.log("[auth-flow][deeplink] handler unmounting");
      if (deepLinkUnsubscribe) {
        deepLinkUnsubscribe();
      }
      unlisten.then((unsubscribes) => {
        unsubscribes.forEach((unsubscribe) => unsubscribe());
      });
    };
  }, [toast]);

  return null; // This component doesn't render anything
}

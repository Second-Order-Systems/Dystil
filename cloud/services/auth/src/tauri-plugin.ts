import { createAuthEndpoint, createAuthMiddleware } from "better-auth/api";
import type { BetterAuthPlugin } from "better-auth";
import type { MiddlewareContext, MiddlewareOptions } from "better-auth";
import type { SocialProvider } from "better-auth/social-providers";

type AuthMiddlewareContext = MiddlewareContext<MiddlewareOptions, any>;

type TauriOptions = {
  callbackURL?: string;
  debugLogs?: boolean;
  scheme: string;
  successText?: string;
  successURL?: string;
};

function appendCallbackURL({
  callbackURL,
  ctx,
  debugLogs,
  scheme,
}: {
  callbackURL: string;
  ctx: AuthMiddlewareContext;
  debugLogs?: boolean;
  scheme: string;
}) {
  if (!ctx.context.options.socialProviders || ctx.path !== "/sign-in/social") {
    return;
  }

  const platform = ctx.request?.headers.get("platform") || "";
  if (platform && ["android", "ios"].includes(platform)) return;

  for (const key of Object.keys(ctx.context.options.socialProviders)) {
    const redirectURI = `${ctx.context.baseURL}/callback/${key}?callbackURL=${scheme}:/${callbackURL}`;
    if (debugLogs) {
      console.log(
        "[Better Auth Tauri] Appending callback URL to social provider",
        key,
        redirectURI,
      );
    }
    ctx.context.options.socialProviders[key as SocialProvider]!.redirectURI = redirectURI;
  }
}

function checkCallbackURL({
  ctx,
  debugLogs,
  scheme,
  successURL,
  url,
}: {
  ctx: AuthMiddlewareContext;
  debugLogs?: boolean;
  scheme: string;
  successURL?: string;
  url: URL;
}) {
  if (!ctx.request) return;

  const searchParams = url.searchParams;
  const callbackURL = searchParams.get("callbackURL");
  if (debugLogs) {
    console.log("[Better Auth Tauri] Callback URL:", callbackURL, url.pathname);
  }
  if (!callbackURL?.startsWith(`${scheme}://`)) return;

  searchParams.set("callbackURL", callbackURL.replace(`${scheme}:/`, ""));
  const deepLinkURL = `${scheme}:/${url.pathname}?${searchParams.toString()}`;
  if (debugLogs) {
    console.log("[Better Auth Tauri] Redirecting to:", deepLinkURL, url.pathname);
  }

  throw ctx.redirect(
    successURL
      ? `${successURL}?redirectTo=${encodeURIComponent(deepLinkURL)}`
      : `${ctx.context.baseURL}/callback/success?redirectTo=${encodeURIComponent(deepLinkURL)}`,
  );
}

const escapeHtml = (value: string) =>
  value.replace(/[&<>"']/g, (character) =>
    ({
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      '"': "&quot;",
      "'": "&#39;",
    })[character]!,
  );

function callbackSuccess(successText: string, scheme: string) {
  return createAuthEndpoint(
    "/callback/success",
    { method: "GET" },
    async (ctx) => {
      if (!ctx.request) return;
      const redirectTo = new URL(ctx.request.url).searchParams.get("redirectTo");
      const hideUI = new URL(ctx.request.url).searchParams.get("hideUI");
      const safeRedirect =
        redirectTo?.startsWith(`${scheme}://`) || redirectTo?.startsWith(`${scheme}:/`)
          ? redirectTo
          : "/";

      return new Response(
        `${hideUI ? "" : `<p>${escapeHtml(successText)}</p>`}<meta http-equiv="refresh" content="0;url=${escapeHtml(safeRedirect)}">`,
        { headers: { "Content-Type": "text/html" } },
      );
    },
  );
}

/** Tauri plugin variant that preserves non-default Better Auth base paths. */
export function tauri(options: TauriOptions): BetterAuthPlugin {
  const {
    callbackURL = "/",
    debugLogs,
    scheme,
    successText = "Your authentication was successful. You may now close this window and return to the application.",
    successURL,
  } = options;

  return {
    id: "tauriPlugin",
    hooks: {
      before: [
        {
          matcher: (context) =>
            !context.request?.url?.includes("/reset-password") &&
            !context.request?.url?.includes("/callback/success"),
          handler: createAuthMiddleware(async (ctx) => {
            if (!ctx.request) return;

            const url = new URL(ctx.request.url);
            if (debugLogs) {
              console.log(
                "[Better Auth Tauri] Request URL:",
                ctx.request.url,
                "User Agent:",
                ctx.request.headers.get("user-agent"),
                "Host:",
                ctx.request.headers.get("host"),
                "Pathname:",
                url.pathname,
              );
            }

            appendCallbackURL({ callbackURL, ctx, debugLogs, scheme });
            checkCallbackURL({ ctx, debugLogs, scheme, successURL, url });
          }),
        },
      ],
    },
    endpoints: {
      getCallbackSuccess: callbackSuccess(successText, scheme),
    },
  };
}

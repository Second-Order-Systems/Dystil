import { createAuthClient } from "better-auth/react";
import { isTauri } from "@tauri-apps/api/core";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";

import { getBuildCapabilities } from "@/lib/build-capabilities";
import { getAuthSessionToken, setAuthSessionToken } from "@/lib/auth-store";

let clientPromise: Promise<ReturnType<typeof createAuthClient>> | null = null;

export function getAuthClient() {
  if (!clientPromise) {
    clientPromise = getBuildCapabilities().then((capabilities) => {
      if (!capabilities.cloudAvailable || !capabilities.cloudBaseUrl) {
        throw new Error("Dystil Cloud is unavailable in this build");
      }
      const authPath =
        capabilities.authMode === "workspace" ? "/api/auth" : "/api/auth/individual";
      return createAuthClient({
        baseURL: `${capabilities.cloudBaseUrl}${authPath}`,
        fetchOptions: {
          auth: {
            type: "Bearer",
            token: () => getAuthSessionToken() ?? "",
          },
          customFetchImpl: (...params) =>
            isTauri() ? tauriFetch(...params) : fetch(...params),
          onSuccess: (ctx) => {
            const token = ctx.response.headers.get("set-auth-token");
            if (token) setAuthSessionToken(token);
          },
        },
      });
    });
  }
  return clientPromise;
}

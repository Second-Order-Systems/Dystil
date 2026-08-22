import { isTauri } from "@tauri-apps/api/core";

import { commands } from "@/lib/utils/tauri";

export type AuthMode = "individual" | "workspace";

export type BuildCapabilities = {
  cloudAvailable: boolean;
  authMode: AuthMode;
  cloudBaseUrl: string | null;
  officialBuild: boolean;
};

const localCapabilities: BuildCapabilities = {
  cloudAvailable: false,
  authMode: "individual",
  cloudBaseUrl: null,
  officialBuild: false,
};

let capabilitiesPromise: Promise<BuildCapabilities> | null = null;

export function getBuildCapabilities(): Promise<BuildCapabilities> {
  if (!isTauri()) return Promise.resolve(localCapabilities);
  if (!capabilitiesPromise) {
    capabilitiesPromise = commands.getBuildCapabilities().catch(
      (error) => {
        console.warn("build capabilities unavailable; using local-only mode", error);
        return localCapabilities;
      },
    );
  }
  return capabilitiesPromise;
}

export function shouldShowWorkEmailGuidance(
  capabilities: Pick<BuildCapabilities, "authMode">,
): boolean {
  return capabilities.authMode === "workspace";
}

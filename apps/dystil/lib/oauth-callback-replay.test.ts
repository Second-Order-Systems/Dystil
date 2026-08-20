import { beforeEach, describe, expect, it } from "vitest";

import {
  claimOAuthCallback,
  isConsumedOAuthStateError,
  markOAuthCallbackProcessed,
  oauthCallbackIdentity,
  releaseOAuthCallbackClaim,
} from "@/lib/oauth-callback-replay";

describe("OAuth callback replay protection", () => {
  beforeEach(() => localStorage.clear());

  it("deduplicates callbacks by state across URL variations", () => {
    const first = oauthCallbackIdentity(
      "dystil://api/auth/callback/google?code=first&state=shared-state",
      "/api/auth/",
    );
    const second = oauthCallbackIdentity(
      "dystil:///api/auth/callback/google?state=shared-state&code=second",
      "/api/auth/",
    );

    expect(first).toBe(second);
    expect(first).not.toContain("shared-state");
  });

  it("keeps a processed callback blocked across remounts until the TTL expires", () => {
    const identity = oauthCallbackIdentity(
      "dystil://api/auth/callback/google?code=secret&state=oauth-state",
      "/api/auth/",
    );

    expect(claimOAuthCallback(identity, 1_000)).toBe(true);
    markOAuthCallbackProcessed(identity, 1_000);
    expect(claimOAuthCallback(identity, 2_000)).toBe(false);
    expect(claimOAuthCallback(identity, 1_000 + 15 * 60 * 1_000 + 1)).toBe(true);
  });

  it("blocks concurrent claims but permits retry after an uncompleted request", () => {
    const identity = "callback";

    expect(claimOAuthCallback(identity, 1_000)).toBe(true);
    expect(claimOAuthCallback(identity, 1_001)).toBe(false);
    releaseOAuthCallbackClaim(identity, 1_002);
    expect(claimOAuthCallback(identity, 1_003)).toBe(true);
  });

  it("recognizes Better Auth's already-consumed state response", () => {
    expect(
      isConsumedOAuthStateError("BetterAuthError: State mismatch: verification not found"),
    ).toBe(true);
    expect(isConsumedOAuthStateError("Google denied access")).toBe(false);
  });
});

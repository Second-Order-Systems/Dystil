import { describe, expect, it } from "vitest";

import { shouldShowWorkEmailGuidance } from "@/lib/build-capabilities";

describe("shouldShowWorkEmailGuidance", () => {
  it("shows advisory guidance for an enterprise build using individual auth", () => {
    expect(
      shouldShowWorkEmailGuidance({
        authMode: "individual",
        enterpriseManaged: true,
      }),
    ).toBe(true);
  });

  it("does not turn an ordinary individual build into a work-email flow", () => {
    expect(
      shouldShowWorkEmailGuidance({
        authMode: "individual",
        enterpriseManaged: false,
      }),
    ).toBe(false);
  });
});

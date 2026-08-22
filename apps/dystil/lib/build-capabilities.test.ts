import { describe, expect, it } from "vitest";

import { shouldShowWorkEmailGuidance } from "@/lib/build-capabilities";

describe("shouldShowWorkEmailGuidance", () => {
  it("shows advisory guidance for workspace authentication", () => {
    expect(shouldShowWorkEmailGuidance({ authMode: "workspace" })).toBe(true);
  });

  it("does not turn an ordinary individual build into a work-email flow", () => {
    expect(
      shouldShowWorkEmailGuidance({ authMode: "individual" }),
    ).toBe(false);
  });
});

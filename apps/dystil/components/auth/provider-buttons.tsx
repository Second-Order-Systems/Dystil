
"use client";

import { useMemo } from "react";

import { cn } from "@/lib/utils";
import { ProviderButton, type SocialLayout } from "./provider-button";

export type ProviderButtonsProps = {
  socialLayout?: SocialLayout;
  isPending: boolean;
  onProvider: (provider: "google" | "github") => void | Promise<void>;
};

export function ProviderButtons({
  socialLayout = "auto",
  isPending,
  onProvider,
}: ProviderButtonsProps) {
  const providers: Array<"google" | "github"> = ["google", "github"];

  const resolvedSocialLayout = useMemo(() => {
    if (socialLayout === "auto") {
      if (providers.length >= 4) {
        return "horizontal";
      }

      return "vertical";
    }

    return socialLayout;
  }, [socialLayout]);

  return (
    <div
      className={cn(
        "gap-3",
        resolvedSocialLayout === "grid" && "grid grid-cols-2",
        resolvedSocialLayout === "vertical" && "flex flex-col",
        resolvedSocialLayout === "horizontal" && "flex flex-row flex-wrap"
      )}
    >
      {providers.map((provider) => (
        <ProviderButton
          key={provider}
          provider={provider}
          display={
            resolvedSocialLayout === "vertical"
              ? "full"
              : resolvedSocialLayout === "grid"
                ? "name"
                : "icon"
          }
          isPending={isPending}
          onClick={() => onProvider(provider)}
          className={cn(resolvedSocialLayout === "horizontal" && "flex-1")}
        />
      ))}
    </div>
  );
}

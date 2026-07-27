
"use client";

import { Github } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";
import { Google } from "./google-icon";

export type SocialLayout = "auto" | "horizontal" | "vertical" | "grid";

export type ProviderButtonProps = {
  provider: "google" | "github";
  display?: "full" | "name" | "icon";
  isPending: boolean;
  onClick: () => void | Promise<void>;
} & Omit<React.ComponentProps<typeof Button>, "onClick" | "children" | "disabled">;

const providerIcons: Record<string, React.ComponentType<{ className?: string }>> = {
  google: Google,
  github: Github,
};

const providerNames: Record<string, string> = {
  google: "Google",
  github: "GitHub",
};

export function ProviderButton({
  provider,
  display = "full",
  variant = "outline",
  isPending,
  onClick,
  className,
  ...props
}: ProviderButtonProps) {
  const ProviderIcon = providerIcons[provider];
  const providerName = providerNames[provider] ?? provider;

  return (
    <Button
      type="button"
      variant={variant}
      className={cn(
        provider === "google" && display === "full"
          ? "relative h-12 justify-center rounded-full px-4 py-3.5"
          : "rounded-full",
        className
      )}
      disabled={isPending}
      onClick={onClick}
      {...props}
      aria-label={`Continue with ${providerName}`}
    >
      {isPending ? (
        <Spinner />
      ) : ProviderIcon ? (
        provider === "google" && display === "full" ? (
          <span className="absolute left-3 top-1/2 inline-flex h-7 w-7 -translate-y-1/2 items-center justify-center rounded-full bg-white shadow-sm">
            <ProviderIcon className="h-[18px] w-[18px] shrink-0" />
          </span>
        ) : (
          <ProviderIcon className="h-4 w-4" />
        )
      ) : null}

      {display === "full"
        ? provider === "google"
          ? <span className="flex-1 text-center">Sign in with Google</span>
          : `Continue with ${providerName}`
        : display === "name"
          ? providerName
          : null}
    </Button>
  );
}

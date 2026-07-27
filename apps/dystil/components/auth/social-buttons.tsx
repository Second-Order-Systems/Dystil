
"use client";

import { Github } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Google } from "./google-icon";

export function SocialButtons({
  disabled,
  onGoogle,
  onGithub,
}: {
  disabled?: boolean;
  onGoogle: () => void | Promise<void>;
  onGithub: () => void | Promise<void>;
}) {
  return (
    <div className="grid gap-2">
      <Button
        type="button"
        variant="outline"
        className="relative h-12 w-full justify-center rounded-full px-4 py-3.5 bg-foreground border"
        disabled={disabled}
        onClick={onGoogle}
      >
        <span className="absolute left-3 top-1/2 inline-flex h-7 w-7 -translate-y-1/2 items-center justify-center rounded-full bg-foreground shadow-sm">
          <Google className="h-[18px] w-[18px] shrink-0" />
        </span>
        <span className="flex-1 text-center">Sign in with Google</span>
      </Button>
      <Button type="button" variant="outline" className="w-full justify-start gap-2 rounded-full" disabled={disabled} onClick={onGithub}>
        <Github className="h-4 w-4" />
        Continue with GitHub
      </Button>
    </div>
  );
}


"use client";

import { LogOut } from "lucide-react";

import { Button } from "@/components/ui/button";
import { signOut } from "@/lib/auth-session";

export function LogoutButton() {
  return (
    <Button type="button" variant="ghost" className="gap-2" onClick={() => void signOut()}>
      <LogOut className="h-4 w-4" />
      Sign out
    </Button>
  );
}

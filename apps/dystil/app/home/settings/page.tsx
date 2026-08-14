"use client";

import { useEffect, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { getVersion } from "@tauri-apps/api/app";
import { SettingsWorkspace } from "@/components/dystil/settings-workspace";
import { toast } from "@/components/ui/use-toast";
import { signOut } from "@/lib/auth-session";
import { useSettings } from "@/lib/hooks/use-settings";

/**
 * Not yet redesigned — Settings has no handoff, and it still brings its own
 * 268px rail. It therefore sits inside the new shell rather than replacing it,
 * and will look transitional until it gets its own design pass.
 */
export default function SettingsPage() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const { settings } = useSettings();
  const [loggingOut, setLoggingOut] = useState(false);
  const [version, setVersion] = useState("");

  useEffect(() => {
    getVersion().then(setVersion).catch(() => {});
  }, []);

  const tab = searchParams.get("tab");
  const initialTab = tab === "Invite your team" ? tab : undefined;

  const logout = async () => {
    setLoggingOut(true);
    try {
      await signOut();
    } catch (error) {
      toast({
        title: "Logout failed",
        description: error instanceof Error ? error.message : String(error),
        variant: "destructive",
      });
    } finally {
      setLoggingOut(false);
    }
  };

  return (
    <SettingsWorkspace
      userName={settings.user?.name?.trim() || "Dystil user"}
      userEmail={settings.user?.email?.trim() || "No email available"}
      onLogout={() => void logout()}
      loggingOut={loggingOut}
      version={version}
      initialTab={initialTab}
      onBack={() => router.push("/home")}
    />
  );
}

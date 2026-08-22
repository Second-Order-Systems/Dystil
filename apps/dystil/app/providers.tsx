"use client";

import { useEffect, useState } from "react";
import { PermissionMonitorProvider } from "@/lib/hooks/use-permission-monitor";
import { DystilSessionProvider } from "@/components/auth/session-provider";
import { Toaster } from "@/components/ui/toaster";
import { DeeplinkHandler } from "@/components/deeplink-handler";
import { SettingsProvider } from "@/lib/hooks/use-settings";
import { AppPolicyProvider } from "@/lib/app-policy";

export function Providers({ children }: { children: React.ReactNode }) {
  const [mounted, setMounted] = useState(false);
  useEffect(() => {
    document.documentElement.setAttribute("data-dystil-mounted", "true");
    try {
      sessionStorage.removeItem("__dystil_startup_recovery");
    } catch (_) {}
    setMounted(true);

    return () => {
      document.documentElement.removeAttribute("data-dystil-mounted");
    };
  }, []);
  return (
    <AppPolicyProvider><SettingsProvider>
      <PermissionMonitorProvider>
        {mounted ? (
          <>
            <DeeplinkHandler />
            <DystilSessionProvider>{children}</DystilSessionProvider>
            <Toaster />
          </>
        ) : null}
      </PermissionMonitorProvider>
    </SettingsProvider></AppPolicyProvider>
  );
}

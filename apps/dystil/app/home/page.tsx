"use client";

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { ChatShell } from "@/components/chat-shell";
import { ToastAction } from "@/components/ui/toast";
import { toast } from "@/components/ui/use-toast";
import { signOut } from "@/lib/auth-session";
import { useSettings } from "@/lib/hooks/use-settings";

type ManagedProvider = "codex" | "claude";
type ProviderStatus = { state: string; authenticated?: boolean | null };
type OnboardingStatus = { aiSetupChoice?: string | null };

const providerName = (provider: ManagedProvider) => provider === "codex" ? "ChatGPT Plus" : "Claude Pro";

export default function HomePage() {
  const { settings } = useSettings();
  const [loggingOut, setLoggingOut] = useState(false);
  const [version, setVersion] = useState("");

  const userName = settings.user?.name?.trim() || "Dystil user";
  const userEmail = settings.user?.email?.trim() || "No email available";

  useEffect(() => {
    getVersion().then(setVersion).catch(() => {});
  }, []);
  useEffect(() => {
    let cancelled = false;
    const resumeProviderSetup = async () => {
      const onboarding = await invoke<OnboardingStatus>("get_onboarding_status").catch(() => null);
      const choice = onboarding?.aiSetupChoice;
      if (choice !== "codex" && choice !== "claude") return;
      const provider = choice as ManagedProvider;
      const name = providerName(provider);
      const status = await invoke<ProviderStatus>("ai_provider_status", { provider }).catch(() => null);
      if (cancelled || status?.authenticated) return;

      const startSignIn = async (notice: ReturnType<typeof toast>) => {
        notice.update({ id: notice.id, title: `${name} sign-in`, description: "Opening your browser…", action: undefined, persistent: true, open: true });
        try {
          const mode = await invoke<string>("ai_provider_login", { provider });
          notice.update({
            id: notice.id,
            title: mode === "codeRequired" ? `${name} needs a code` : `${name} sign-in is open`,
            description: mode === "codeRequired" ? "Finish in your browser, then paste the authorization code in Settings." : "Finish in your browser. Dystil will recognize the connection when you return.",
            action: undefined,
            persistent: true,
            open: true,
          });
        } catch (error) {
          notice.update({ id: notice.id, title: `${name} sign-in needs attention`, description: error instanceof Error ? error.message : String(error), variant: "destructive", persistent: true, open: true });
        }
      };

      if (status?.state === "ready") {
        const notice = toast({
          title: `${name} is ready to connect`,
          description: "Sign in to use Ask Your Work.",
          action: <ToastAction altText={`Sign in to ${name}`} onClick={() => void startSignIn(notice)}>Sign in</ToastAction>,
          persistent: true,
          showWhenNotificationsDisabled: true,
        });
        return;
      }

      const notice = toast({
        title: `${name} setup needs attention`,
        description: "Dystil could not finish preparing the private connection.",
        persistent: true,
        variant: "destructive",
        showWhenNotificationsDisabled: true,
      });
      notice.update({
        id: notice.id,
        action: <ToastAction altText={`Retry ${name} setup`} className="border-current bg-transparent text-current hover:bg-background/15" onClick={() => void (async () => {
          notice.update({ id: notice.id, title: `${name} is installing privately`, description: undefined, action: undefined, persistent: true, open: true });
          try {
            await invoke("ai_provider_install", { provider });
            notice.update({ id: notice.id, title: `${name} is ready to connect`, description: "Sign in to use Ask Your Work.", action: <ToastAction altText={`Sign in to ${name}`} onClick={() => void startSignIn(notice)}>Sign in</ToastAction>, variant: "default", persistent: true, open: true });
          } catch (error) {
            notice.update({ id: notice.id, title: `${name} setup needs attention`, description: error instanceof Error ? error.message : String(error), variant: "destructive", persistent: true, open: true });
          }
        })()}>Retry setup</ToastAction>,
      });
    };
    void resumeProviderSetup();
    return () => { cancelled = true; };
  }, []);

  const logout = async () => { setLoggingOut(true); try { await signOut(); } catch (error) { toast({ title: "Logout failed", description: error instanceof Error ? error.message : String(error), variant: "destructive" }); } finally { setLoggingOut(false); } };

  return <ChatShell
    userName={userName} userEmail={userEmail}
    onLogout={() => void logout()} loggingOut={loggingOut} version={version}
  />;
}

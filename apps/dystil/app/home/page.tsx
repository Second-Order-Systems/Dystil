"use client";

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { ChatShell, type Chat, type ChatSession } from "@/components/chat-shell";
import { ToastAction } from "@/components/ui/toast";
import { toast } from "@/components/ui/use-toast";
import { signOut } from "@/lib/auth-session";
import { useSettings } from "@/lib/hooks/use-settings";
import { commands, type LocalChatMessageView } from "@/lib/utils/tauri";

type AgentPeer = { userId: string; displayName: string | null; email: string; agentStatus: string };
type AgentMessage = { messageId: string; peerUserId: string; direction: string; kind: string; localStatus: string; text: string; evidence: Array<{ label: string; localDate: string }> };
type ManagedProvider = "codex" | "claude";
type ProviderStatus = { state: string; authenticated?: boolean | null };
type OnboardingStatus = { aiSetupChoice?: string | null };

const providerName = (provider: ManagedProvider) => provider === "codex" ? "ChatGPT Plus" : "Claude Pro";

function parseCitations(value?: string | null): Chat["citations"] {
  if (!value) return [];
  try {
    const parsed: unknown = JSON.parse(value);
    if (!Array.isArray(parsed)) return [];
    return parsed.flatMap((item) => {
      if (!item || typeof item !== "object") return [];
      const citation = item as Record<string, unknown>;
      if (typeof citation.label !== "string") return [];
      const localDate = citation.localDate ?? citation.local_date;
      return [{ label: citation.label, localDate: typeof localDate === "string" ? localDate : "" }];
    });
  } catch {
    return [];
  }
}

function toChatTurns(messages: LocalChatMessageView[]): Chat[] {
  const turns: Chat[] = [];
  for (let index = 0; index < messages.length; index += 1) {
    const user = messages[index];
    if (user.role !== "user" || !user.question) continue;
    const nextMessage = messages[index + 1];
    const assistant = nextMessage?.role === "assistant" ? nextMessage : undefined;
    turns.push({
      id: assistant?.id || user.id,
      conversationId: user.sessionId,
      question: user.question,
      mode: user.mode === "team" ? "team" : "local",
      answer: assistant?.answer,
      status: (assistant?.status as Chat["status"]) || "failed",
      citations: parseCitations(assistant?.citationsJson),
      provider: assistant?.provider,
      model: assistant?.model,
      elapsedMs: assistant?.elapsedMs,
      historical: true,
    });
  }
  return turns;
}

export default function HomePage() {
  const { settings } = useSettings();
  const [loggingOut, setLoggingOut] = useState(false);
  const [version, setVersion] = useState("");
  const [peers, setPeers] = useState<AgentPeer[]>([]);
  const [agentMessages, setAgentMessages] = useState<AgentMessage[]>([]);
  const [sessions, setSessions] = useState<ChatSession[]>([]);

  const userName = settings.user?.name?.trim() || "Dystil user";
  const userEmail = settings.user?.email?.trim() || "No email available";
  const refreshMailbox = async () => {
    const [nextPeers, nextMessages] = await Promise.all([
      invoke<AgentPeer[]>("agent_list_peers").catch(() => []),
      invoke<AgentMessage[]>("agent_list_messages").catch(() => []),
    ]);
    setPeers(nextPeers); setAgentMessages(nextMessages);
  };
  const refreshSessions = async () => {
    const result = await commands.localChatListSessions();
    // Do not let Fast Refresh retain a previous implementation's in-memory
    // history when the currently running desktop binary cannot serve sessions.
    setSessions(result.status === "ok" ? result.data : []);
  };

  useEffect(() => {
    getVersion().then(setVersion).catch(() => {});
  }, []);
  useEffect(() => { void refreshSessions(); }, []);
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
  useEffect(() => {
    void refreshMailbox(); let unlisten: (() => void) | undefined;
    listen("agent-mailbox-updated", () => void refreshMailbox()).then((dispose) => { unlisten = dispose; }).catch(() => {});
    return () => unlisten?.();
  }, []);

  const logout = async () => { setLoggingOut(true); try { await signOut(); } catch (error) { toast({ title: "Logout failed", description: error instanceof Error ? error.message : String(error), variant: "destructive" }); } finally { setLoggingOut(false); } };

  return <ChatShell
    userName={userName} userEmail={userEmail}
    peers={peers} agentMessages={agentMessages} sessions={sessions}
    onLoadSession={async (sessionId) => {
      const result = await commands.localChatGetMessages(sessionId);
      if (result.status === "error") throw new Error(result.error);
      return toChatTurns(result.data);
    }}
    onSendLocal={async (sessionId, question) => {
      const result = await commands.localChatSend(sessionId, question);
      if (result.status === "error") throw new Error(result.error);
      await refreshSessions();
      return { id: result.data.id, conversationId: sessionId, question, mode: "local", answer: result.data.answer, status: result.data.status as Chat["status"], citations: parseCitations(result.data.citationsJson), provider: result.data.provider, model: result.data.model, elapsedMs: result.data.elapsedMs };
    }}
    onAskPeer={async (recipientUserId, question) => { await invoke("agent_send_question", { recipientUserId, question }); await refreshMailbox(); }}
    onLogout={() => void logout()} loggingOut={loggingOut} version={version}
  />;
}

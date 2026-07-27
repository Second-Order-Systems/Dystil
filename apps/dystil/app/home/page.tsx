"use client";

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { ChatShell, type Chat, type ChatSession } from "@/components/chat-shell";
import { toast } from "@/components/ui/use-toast";
import { signOut } from "@/lib/auth-session";
import { useHealthCheck } from "@/lib/hooks/use-health-check";
import { useSettings } from "@/lib/hooks/use-settings";
import { requestPermissionWithFlow } from "@/lib/utils/permission-flow";
import { commands, type LocalChatMessageView, type WorkCardView } from "@/lib/utils/tauri";

type AgentPeer = { userId: string; displayName: string | null; email: string; agentStatus: string };
type AgentMessage = { messageId: string; peerUserId: string; direction: string; kind: string; localStatus: string; text: string; evidence: Array<{ label: string; localDate: string }> };

function toChatTurns(messages: LocalChatMessageView[]): Chat[] {
  const turns: Chat[] = [];
  for (let index = 0; index < messages.length; index += 1) {
    const user = messages[index];
    if (user.role !== "user" || !user.question) continue;
    const assistant = messages.slice(index + 1).find((message) => message.role === "assistant");
    let citations: Chat["citations"];
    try { citations = assistant?.citationsJson ? JSON.parse(assistant.citationsJson) : []; } catch { citations = []; }
    turns.push({
      id: assistant?.id || user.id,
      conversationId: user.sessionId,
      question: user.question,
      mode: user.mode === "team" ? "team" : "local",
      answer: assistant?.answer,
      status: (assistant?.status as Chat["status"]) || "failed",
      citations,
      provider: assistant?.provider,
      model: assistant?.model,
      elapsedMs: assistant?.elapsedMs,
    });
  }
  return turns;
}

async function isCaptureRunning() { return invoke<boolean>("is_capture_running").catch(() => false); }

export default function HomePage() {
  const { settings, reloadStore } = useSettings();
  const { health, isServerDown, fetchHealth } = useHealthCheck();
  const [captureRunning, setCaptureRunning] = useState<boolean | null>(null);
  const [toggling, setToggling] = useState(false);
  const [screenshotBusy, setScreenshotBusy] = useState(false);
  const [loggingOut, setLoggingOut] = useState(false);
  const [version, setVersion] = useState("");
  const [cards, setCards] = useState<WorkCardView[]>([]);
  const [loadingCards, setLoadingCards] = useState(true);
  const [peers, setPeers] = useState<AgentPeer[]>([]);
  const [agentMessages, setAgentMessages] = useState<AgentMessage[]>([]);
  const [sessions, setSessions] = useState<ChatSession[]>([]);

  const recording = isServerDown || health?.status_code === 500 ? false : captureRunning ?? false;
  const userName = settings.user?.name?.trim() || "Dystil user";
  const userEmail = settings.user?.email?.trim() || "No email available";
  const refreshCapture = async () => setCaptureRunning(await isCaptureRunning());
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
    void refreshCapture(); getVersion().then(setVersion).catch(() => {});
    let unlisten: (() => void) | undefined;
    listen("recording-status-changed", () => void refreshCapture()).then((dispose) => { unlisten = dispose; }).catch(() => {});
    return () => unlisten?.();
  }, []);
  useEffect(() => { void refreshSessions(); }, []);
  useEffect(() => {
    void refreshMailbox(); let unlisten: (() => void) | undefined;
    listen("agent-mailbox-updated", () => void refreshMailbox()).then((dispose) => { unlisten = dispose; }).catch(() => {});
    return () => unlisten?.();
  }, []);
  useEffect(() => {
    let cancelled = false; const timer = window.setTimeout(async () => {
      setLoadingCards(true); const result = await commands.searchWorkCards("", 120);
      if (!cancelled && result.status === "ok") setCards(result.data);
      if (!cancelled) setLoadingCards(false);
    }, 180);
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [captureRunning]);

  const toggleCapture = async () => {
    setToggling(true); const target = !recording;
    try {
      const result = target ? await commands.startCapture() : await commands.stopCapture();
      if (result.status === "error") throw new Error(result.error);
      await new Promise((resolve) => window.setTimeout(resolve, 300));
      await refreshCapture(); await fetchHealth();
    } catch (error) { toast({ title: "Could not update recording", description: error instanceof Error ? error.message : String(error), variant: "destructive" }); }
    finally { setToggling(false); }
  };
  const setScreenshots = async (enabled: boolean) => {
    setScreenshotBusy(true);
    try {
      if (enabled) {
        let permission = await commands.checkScreenRecordingPermission();
        if (permission !== "granted" && permission !== "notNeeded") { await requestPermissionWithFlow("screenRecording"); permission = await commands.checkScreenRecordingPermission(); }
        if (permission !== "granted" && permission !== "notNeeded") throw new Error("Screen Recording permission was not granted.");
      }
      const result = await commands.setScreenshotCaptureEnabled(enabled);
      if (result.status === "error") throw new Error(result.error);
      await reloadStore(); await refreshCapture();
    } catch (error) { toast({ title: "Could not update screenshot capture", description: error instanceof Error ? error.message : String(error), variant: "destructive" }); }
    finally { setScreenshotBusy(false); }
  };
  const logout = async () => { setLoggingOut(true); try { await signOut(); } catch (error) { toast({ title: "Logout failed", description: error instanceof Error ? error.message : String(error), variant: "destructive" }); } finally { setLoggingOut(false); } };

  return <ChatShell
    userName={userName} userEmail={userEmail} recording={recording} toggling={toggling} onToggleRecording={() => void toggleCapture()}
    screenshotEnabled={!settings.disableVision} onScreenshotChange={(enabled) => void setScreenshots(enabled)} screenshotBusy={screenshotBusy}
    peers={peers} agentMessages={agentMessages} cards={cards} loadingCards={loadingCards} sessions={sessions}
    onLoadSession={async (sessionId) => {
      const result = await commands.localChatGetMessages(sessionId);
      if (result.status === "error") throw new Error(result.error);
      return toChatTurns(result.data);
    }}
    onSendLocal={async (sessionId, question) => {
      const result = await commands.localChatSend(sessionId, question);
      if (result.status === "error") throw new Error(result.error);
      await refreshSessions();
      let citations: Chat["citations"];
      try { citations = result.data.citationsJson ? JSON.parse(result.data.citationsJson) : []; } catch { citations = []; }
      return { id: result.data.id, conversationId: sessionId, question, mode: "local", answer: result.data.answer, status: result.data.status as Chat["status"], citations, provider: result.data.provider, model: result.data.model, elapsedMs: result.data.elapsedMs };
    }}
    onAskPeer={async (recipientUserId, question) => { await invoke("agent_send_question", { recipientUserId, question }); await refreshMailbox(); }}
    onLogout={() => void logout()} loggingOut={loggingOut} version={version}
  />;
}

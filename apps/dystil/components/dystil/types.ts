export type Peer = { userId: string; displayName: string | null; email: string; agentStatus: string };
export type AgentMessage = {
  messageId: string;
  peerUserId: string;
  direction: string;
  kind: string;
  localStatus: string;
  text: string;
  evidence: Array<{ label: string; localDate: string }>;
};

export type ChatSession = { id: string; title: string; updatedAt: string };
export type Chat = {
  id: string;
  conversationId: string;
  question: string;
  mode: "local" | "team";
  answer?: string | null;
  status?: "pending" | "complete" | "failed";
  citations?: Array<{ label: string; localDate: string }>;
  provider?: string | null;
  model?: string | null;
  elapsedMs?: number | null;
  historical?: boolean;
};

export type SettingsTab =
  | "What Dystil can see"
  | "AI models"
  | "When it runs"
  | "Storage"
  | "Notifications"
  | "Invite your team"
  | "About";

export type DystilShellProps = {
  userName: string;
  userEmail: string;
  peers: Peer[];
  agentMessages: AgentMessage[];
  sessions: ChatSession[];
  onLoadSession: (sessionId: string) => Promise<Chat[]>;
  onSendLocal: (sessionId: string, question: string) => Promise<Chat>;
  onAskPeer: (peerId: string, question: string) => Promise<void>;
  onLogout: () => void;
  loggingOut: boolean;
  version: string;
};

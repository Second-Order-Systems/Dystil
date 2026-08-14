export type SettingsTab =
  | "What Dystil can see"
  | "AI models"
  | "When it runs"
  | "Storage"
  | "Notifications"
  | "Invite your team"
  | "About";

/**
 * Props the shell actually renders. It accepts nothing it does not use — the
 * previous version carried peers, agentMessages, sessions and three async
 * callbacks that no component read.
 */
export type DystilShellProps = {
  userName: string;
  userEmail: string;
  onLogout: () => void;
  loggingOut: boolean;
  version: string;
};

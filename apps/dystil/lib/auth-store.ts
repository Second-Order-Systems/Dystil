
export type DystilUserSession = {
  session_token: string | null;
  expires_at: string | null;
};

export type DystilUserOrg = {
  id: string;
  name: string | null;
  slug: string | null;
  roles: string[];
};

export type DystilUserProfile = {
  id: string;
  email: string | null;
  name: string | null;
  image: string | null;
  org: DystilUserOrg | null;
};

export type DystilAuthState = {
  status:
    | "signed_out"
    | "authenticating"
    | "awaiting_email_verification"
    | "session_ready"
    | "profile_loading"
    | "device_registering"
    | "ready"
    | "error";
  session: DystilUserSession | null;
  user: DystilUserProfile | null;
  device_token_present: boolean;
  error: string | null;
  pending_verification_email: string | null;
};

type Listener = (state: DystilAuthState) => void;

let authSessionToken: string | null = null;
let authState: DystilAuthState = {
  status: "signed_out",
  session: null,
  user: null,
  device_token_present: false,
  error: null,
  pending_verification_email: null,
};
const listeners = new Set<Listener>();

export function getAuthSessionToken() {
  return authSessionToken;
}

export function setAuthSessionToken(token: string | null) {
  authSessionToken = token;
}

export function getAuthState() {
  return authState;
}

export function setAuthState(next: DystilAuthState) {
  authState = next;
  for (const listener of listeners) {
    listener(authState);
  }
}

export function subscribeAuthState(listener: Listener) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function setAwaitingEmailVerification(email: string) {
  setAuthState({
    status: "awaiting_email_verification",
    session: null,
    user: null,
    device_token_present: false,
    error: null,
    pending_verification_email: email,
  });
}

export function resetToSignedOut() {
  authSessionToken = null;
  setAuthState({
    status: "signed_out",
    session: null,
    user: null,
    device_token_present: false,
    error: null,
    pending_verification_email: null,
  });
}

export function clearAuthState() {
  resetToSignedOut();
}

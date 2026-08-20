const STORAGE_KEY = "dystil.oauth.processed-callbacks.v1";
const PROCESSED_TTL_MS = 15 * 60 * 1000;
const CLAIM_TTL_MS = 60 * 1000;

type CallbackRecord = {
  expiresAt: number;
  status: "processing" | "processed";
};

type CallbackRecords = Record<string, CallbackRecord>;

function hash(value: string): string {
  let result = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    result ^= value.charCodeAt(index);
    result = Math.imul(result, 16777619);
  }
  return (result >>> 0).toString(36);
}

function readRecords(now: number): CallbackRecords {
  try {
    const parsed = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}") as CallbackRecords;
    return Object.fromEntries(
      Object.entries(parsed).filter(
        ([, record]) =>
          record &&
          (record.status === "processing" || record.status === "processed") &&
          Number.isFinite(record.expiresAt) &&
          record.expiresAt > now,
      ),
    );
  } catch {
    return {};
  }
}

function writeRecords(records: CallbackRecords) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(records));
  } catch {
    // Replay protection still works within this renderer through the in-memory
    // guard in DeeplinkHandler. Storage can be unavailable in hardened webviews.
  }
}

export function oauthCallbackIdentity(url: string, authBasePath: string): string {
  try {
    const state = new URL(url).searchParams.get("state");
    if (state) return `${authBasePath}:state:${hash(state)}`;
  } catch {
    // Fall back to a hash of the URL below.
  }
  return `${authBasePath}:url:${hash(url)}`;
}

export function claimOAuthCallback(identity: string, now = Date.now()): boolean {
  const records = readRecords(now);
  if (records[identity]) return false;
  records[identity] = { status: "processing", expiresAt: now + CLAIM_TTL_MS };
  writeRecords(records);
  return true;
}

export function markOAuthCallbackProcessed(identity: string, now = Date.now()) {
  const records = readRecords(now);
  records[identity] = { status: "processed", expiresAt: now + PROCESSED_TTL_MS };
  writeRecords(records);
}

export function releaseOAuthCallbackClaim(identity: string, now = Date.now()) {
  const records = readRecords(now);
  if (records[identity]?.status !== "processing") return;
  delete records[identity];
  writeRecords(records);
}

export function isConsumedOAuthStateError(body: string): boolean {
  return /state mismatch/i.test(body) && /verification not found|already consumed/i.test(body);
}

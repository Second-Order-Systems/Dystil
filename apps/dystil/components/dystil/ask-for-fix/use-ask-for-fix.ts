"use client";

import { useCallback, useEffect, useState } from "react";

import {
  commands,
  type AskInputEvent,
  type AskSessionView,
  type AskUserTurn,
} from "@/lib/utils/tauri";

type CommandResult<T> =
  | { status: "ok"; data: T }
  | { status: "error"; error: string };

function unwrap<T>(result: CommandResult<T>): T {
  if (result.status === "error") throw new Error(result.error);
  return result.data;
}

function readableError(error: unknown) {
  const message = error instanceof Error ? error.message : String(error);
  if (message.includes("provider_not_ready") || message.includes("not ready")) {
    return "Connect an AI model in Settings, then try this turn again.";
  }
  if (message.includes("authentication")) {
    return "Your model sign-in needs attention. Reconnect it in Settings, then retry.";
  }
  if (message.includes("timeout")) {
    return "The model took too long to answer. Your conversation is safe; try again.";
  }
  if (message.includes("invalid_output")) {
    return "Dystil could not shape a valid response after one repair. Try the turn again.";
  }
  if (message.includes("user_cancelled")) {
    return "You stopped the response. Your conversation is still here.";
  }
  if (message.includes("interrupted")) {
    return "The app closed before this response finished. Your conversation is safe; try the turn again.";
  }
  return message || "Dystil could not finish this turn. Your conversation is still here.";
}

function sessionError(session: AskSessionView) {
  if (!session.lastErrorCode) return null;
  return readableError(`${session.lastErrorCode}: ${session.lastErrorDetail ?? ""}`);
}

export function useAskForFix() {
  const [session, setSession] = useState<AskSessionView | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [optimisticText, setOptimisticText] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    void commands
      .askForFixLatest()
      .then(unwrap)
      .then((latest) => {
        if (alive) {
          setSession(latest);
          setError(latest ? sessionError(latest) : null);
        }
      })
      .catch((failure) => {
        if (alive) setError(readableError(failure));
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, []);

  const ensureSession = useCallback(async () => {
    if (session) return session;
    const created = unwrap(await commands.askForFixCreate());
    setSession(created);
    return created;
  }, [session]);

  const run = useCallback(async (operation: () => Promise<AskSessionView>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const next = await operation();
      setSession(next);
      setError(sessionError(next));
    } catch (failure) {
      setError(readableError(failure));
    } finally {
      setBusy(false);
      setOptimisticText(null);
    }
  }, [busy]);

  const submit = useCallback(async (text: string, event: AskInputEvent) => {
    const normalized = text.trim();
    if (!normalized || busy) return;
    setOptimisticText(normalized);
    await run(async () => {
      const active = await ensureSession();
      const turn: AskUserTurn = { text: normalized, event };
      return unwrap(await commands.askForFixSubmit(active.sessionId, turn));
    });
  }, [busy, ensureSession, run]);

  const confirm = useCallback(async () => {
    if (!session || busy) return;
    setOptimisticText("Solve this.");
    await run(async () => unwrap(await commands.askForFixConfirm(session.sessionId)));
  }, [busy, run, session]);

  const retry = useCallback(async () => {
    if (!session || busy) return;
    await run(async () => unwrap(await commands.askForFixRetry(session.sessionId)));
  }, [busy, run, session]);

  const cancel = useCallback(async () => {
    if (!session || !busy) return;
    try {
      const next = unwrap(await commands.askForFixCancel(session.sessionId));
      setSession(next);
      setError("You stopped the response. Your conversation is still here.");
    } catch (failure) {
      setError(readableError(failure));
    } finally {
      setBusy(false);
      setOptimisticText(null);
    }
  }, [busy, session]);

  const keepArtifact = useCallback(async () => {
    if (!session || busy) return;
    setBusy(true);
    setError(null);
    try {
      const artifactId = unwrap(await commands.askForFixKeepArtifact(session.sessionId));
      setSession({ ...session, artifactKeptId: artifactId });
    } catch (failure) {
      setError(readableError(failure));
    } finally {
      setBusy(false);
    }
  }, [busy, session]);

  const startWatching = useCallback(async () => {
    if (!session || busy) return;
    await run(async () => unwrap(await commands.askForFixStartWatching(session.sessionId)));
  }, [busy, run, session]);

  const stopWatching = useCallback(async () => {
    if (!session || busy) return;
    await run(async () => unwrap(await commands.askForFixStopWatching(session.sessionId)));
  }, [busy, run, session]);

  const reviewWatch = useCallback(async () => {
    if (!session || busy) return;
    setOptimisticText("Review what Dystil found.");
    await run(async () => unwrap(await commands.askForFixReviewWatch(session.sessionId)));
  }, [busy, run, session]);

  const updateWatchGuidance = useCallback(async (guidance: string) => {
    if (!session || busy || !guidance.trim()) return;
    await run(async () => unwrap(await commands.askForFixUpdateWatchGuidance(session.sessionId, guidance.trim())));
  }, [busy, run, session]);

  const startNew = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      setSession(unwrap(await commands.askForFixCreate()));
    } catch (failure) {
      setError(readableError(failure));
    } finally {
      setBusy(false);
    }
  }, [busy]);

  return {
    session,
    loading,
    busy,
    error,
    optimisticText,
    submit,
    confirm,
    retry,
    cancel,
    keepArtifact,
    startWatching,
    stopWatching,
    reviewWatch,
    updateWatchGuidance,
    startNew,
  };
}

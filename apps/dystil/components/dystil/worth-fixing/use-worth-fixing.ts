"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type DispositionKind, type WorthFixingCard, type WorthFixingSummary } from "@/lib/utils/tauri";

export function useWorthFixing() {
  const [summary, setSummary] = useState<WorthFixingSummary | null>(null);
  const [other, setOther] = useState<WorthFixingCard[]>([]);
  const [otherCursor, setOtherCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const request = useRef(0);

  const load = useCallback(async () => {
    const current = ++request.current;
    setLoading(true);
    const result = await commands.getWorthFixingSummary();
    if (current !== request.current) return;
    setLoading(false);
    if (result.status === "error") return setError(result.error);
    setSummary(result.data);
    setError(null);
  }, []);

  useEffect(() => {
    void load();
    const onFocus = () => void load();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [load]);

  const keep = useCallback(async (findingId: string) => {
    if (pendingId) return null;
    setPendingId(findingId);
    const result = await commands.keepWorthFixingFinding(findingId);
    setPendingId(null);
    if (result.status === "error") {
      setError(result.error);
      await load();
      return null;
    }
    setSummary(result.data.summary);
    setNotice(`Kept “${result.data.artifact.title}”.`);
    setError(null);
    return result.data.artifact.artifactId;
  }, [load, pendingId]);

  const dismiss = useCallback(async (findingId: string, reason: DispositionKind) => {
    if (pendingId) return;
    setPendingId(findingId);
    const result = await commands.dismissWorthFixingFinding(findingId, reason);
    setPendingId(null);
    if (result.status === "error") setError(result.error);
    else {
      setNotice("Removed from Worth fixing.");
      await load();
    }
  }, [load, pendingId]);

  const correct = useCallback(async (findingId: string, correctionText: string, intent: string) => {
    if (pendingId) return false;
    setPendingId(findingId);
    const result = await commands.correctWorthFixingFinding(findingId, correctionText, intent);
    setPendingId(null);
    if (result.status === "error") {
      setError(result.error);
      return false;
    }
    setNotice("Your correction was saved.");
    await load();
    return true;
  }, [load, pendingId]);

  const refresh = useCallback(async () => {
    setLoading(true);
    const result = await commands.refreshWorthFixing();
    if (result.status === "error") setError(result.error);
    await load();
  }, [load]);

  const loadOther = useCallback(async () => {
    const result = await commands.getOtherWorthFixingFindings(otherCursor, 10);
    if (result.status === "error") return setError(result.error);
    setOther((current) => [...current, ...result.data.items.filter((item) => !current.some((old) => old.findingId === item.findingId))]);
    setOtherCursor(result.data.nextCursor);
  }, [otherCursor]);

  return { summary, other, otherCursor, loading, error, pendingId, notice, load, keep, dismiss, correct, refresh, loadOther };
}

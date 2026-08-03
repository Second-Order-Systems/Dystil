"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type ArtifactChangePreview, type ReadyArtifactAction, type ReadyArtifactCard, type ReadyArtifactDetail, type WorthFixingEvidenceLine } from "@/lib/utils/tauri";

export function useReadyArtifacts() {
  const [items, setItems] = useState<ReadyArtifactCard[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [detail, setDetail] = useState<ReadyArtifactDetail | null>(null);
  const [provenance, setProvenance] = useState<WorthFixingEvidenceLine[] | null>(null);
  const [preview, setPreview] = useState<ArtifactChangePreview | null>(null);
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [waitingCount, setWaitingCount] = useState(0);
  const request = useRef(0);

  const load = useCallback(async () => {
    const current = ++request.current;
    setLoading(true);
    const [result, worth] = await Promise.all([
      commands.getReadyToUse(null, 50),
      commands.getWorthFixingSummary(),
    ]);
    if (current !== request.current) return;
    setLoading(false);
    if (result.status === "error") return setError(result.error);
    setItems(result.data.items);
    setCursor(result.data.nextCursor);
    if (worth.status === "ok") setWaitingCount(worth.data.eligibleCount);
    setError(null);
  }, []);

  useEffect(() => {
    void load();
    const onFocus = () => void load();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [load]);

  const open = useCallback(async (artifact: ReadyArtifactCard, action: ReadyArtifactAction = "open") => {
    setPending(artifact.artifactId);
    if (artifact.kind === "existing_capability" && action === "open") {
      const result = await commands.openReadyCapability(artifact.artifactId);
      setPending(null);
      if (result.status === "error") setError(result.error); else { setNotice(`Opened “${artifact.title}”.`); await load(); }
      return;
    }
    const result = await commands.getReadyArtifact(artifact.artifactId);
    if (result.status === "error") setError(result.error);
    else {
      setDetail(result.data);
      setPreview(null);
      setProvenance(null);
      const receipt = await commands.recordReadyArtifactUsed(artifact.artifactId, action);
      if (receipt.status === "error") setError(receipt.error);
    }
    setPending(null);
  }, [load]);

  const copy = useCallback(async (artifact: ReadyArtifactCard) => {
    setPending(artifact.artifactId);
    const body = detail?.card.artifactId === artifact.artifactId ? detail.body : await commands.getReadyArtifact(artifact.artifactId).then((result) => result.status === "ok" ? result.data.body : Promise.reject(new Error(result.error)));
    try {
      await navigator.clipboard.writeText(body);
      const receipt = await commands.recordReadyArtifactUsed(artifact.artifactId, "copy");
      if (receipt.status === "error") throw new Error(receipt.error);
      setNotice(`Copied “${artifact.title}”.`);
      await load();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "The artifact could not be copied.");
    } finally { setPending(null); }
  }, [detail, load]);

  const share = useCallback(async (artifact: ReadyArtifactCard) => {
    setPending(artifact.artifactId);
    const result = await commands.getReadyArtifact(artifact.artifactId);
    if (result.status === "error") { setPending(null); return setError(result.error); }
    try {
      const nativeShare = "share" in navigator;
      if (nativeShare) await navigator.share({ title: artifact.title, text: result.data.body });
      else await navigator.clipboard.writeText(`${artifact.title}\n\n${result.data.body}`);
      const receipt = await commands.recordReadyArtifactUsed(artifact.artifactId, "share");
      if (receipt.status === "error") throw new Error(receipt.error);
      setNotice(nativeShare ? "Share sheet opened." : "Copied so you can send it.");
      await load();
    } catch (cause) {
      if (cause instanceof DOMException && cause.name === "AbortError") return;
      setError(cause instanceof Error ? cause.message : "The artifact could not be shared.");
    } finally { setPending(null); }
  }, [load]);

  const showProvenance = useCallback(async (artifactId: string) => {
    const result = await commands.getReadyArtifactProvenance(artifactId);
    if (result.status === "error") setError(result.error); else setProvenance(result.data);
  }, []);

  const propose = useCallback(async (artifactId: string, change: string) => {
    setPending(`change:${artifactId}`);
    const result = await commands.proposeReadyArtifactChange(artifactId, change);
    setPending(null);
    if (result.status === "error") { setError(result.error); return false; }
    setPreview(result.data); setError(null); return true;
  }, []);

  const retry = useCallback(async () => {
    if (!preview) return;
    setPending(`change:${preview.artifactId}`);
    const result = await commands.retryReadyArtifactChange(preview.changeJobId);
    setPending(null);
    if (result.status === "error") setError(result.error); else setPreview(result.data);
  }, [preview]);

  const confirm = useCallback(async () => {
    if (!preview) return;
    setPending(`change:${preview.artifactId}`);
    const result = await commands.confirmReadyArtifactChange(preview.changeJobId);
    setPending(null);
    if (result.status === "error") return setError(result.error);
    setDetail(result.data); setPreview(null); setNotice("Change saved."); await load();
  }, [load, preview]);

  const reject = useCallback(async () => {
    if (!preview) return;
    const result = await commands.rejectReadyArtifactChange(preview.changeJobId);
    if (result.status === "error") return setError(result.error);
    setDetail(result.data); setPreview(null); setNotice("Left it as it was.");
  }, [preview]);

  const remove = useCallback(async (artifactId: string) => {
    setPending(artifactId);
    const result = await commands.removeReadyArtifact(artifactId);
    setPending(null);
    if (result.status === "error") return setError(result.error);
    setDetail(null); setPreview(null); setNotice("Artifact removed."); await load();
  }, [load]);

  return { items, cursor, waitingCount, detail, provenance, preview, loading, pending, error, notice, load, open, copy, share, showProvenance, propose, retry, confirm, reject, remove, close: () => { setDetail(null); setPreview(null); setProvenance(null); } };
}

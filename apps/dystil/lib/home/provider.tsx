"use client";

import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";

import { commands, type DispositionKind, type HomeWorthFixingItem, type ReadyArtifactAction, type ReadyArtifactCard, type SkillBundleView } from "@/lib/utils/tauri";
import { getBuildCapabilities } from "@/lib/build-capabilities";

import type { CorrectionReason, HomeItem, HomeSource, Shortcut } from "./types";

const HomeContext = createContext<HomeSource | null>(null);

function relativeWhen(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "recently";
  const days = Math.floor((Date.now() - date.getTime()) / 86_400_000);
  if (days <= 0) return "today";
  if (days === 1) return "yesterday";
  if (days < 7) return `${days} days ago`;
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function itemFrom(value: HomeWorthFixingItem): HomeItem {
  return {
    id: value.findingId,
    origin: "dystil",
    when: relativeWhen(value.occurredAt),
    short: value.title,
    title: value.title,
    evidence: value.evidence.map((stat) => ({ n: stat.value, label: stat.label })),
    evidenceNote: value.evidenceNote,
    offer: value.offer,
    fixName: value.fixName,
    steps: value.steps.map((step, index) => ({ n: String(index + 1), t: step })),
    saveAvailable: value.saveAvailable,
  };
}

function shortcutFrom(value: ReadyArtifactCard): Shortcut {
  return {
    id: value.artifactId,
    title: value.title,
    meta: value.lastUsedAt ? `Used ${relativeWhen(value.lastUsedAt)}` : "Saved for later",
    kind: value.kind.replaceAll("_", " "),
  };
}

function withBundle(shortcut: Shortcut, bundle?: SkillBundleView): Shortcut {
  if (!bundle) return shortcut;
  return { ...shortcut, bundle: { bundleId: bundle.bundleId ?? undefined, skillName: bundle.skillName ?? undefined, status: bundle.status, stage: bundle.stage ?? undefined, error: bundle.errorMessage ?? undefined } };
}

function dispositionFor(reason: CorrectionReason): DispositionKind {
  if (reason === "intended") return "not_a_problem";
  if (reason === "not-worth-it") return "leave_it";
  return "close_but";
}

export function HomeProvider({ children }: { children: React.ReactNode }) {
  const [items, setItems] = useState<HomeItem[]>([]);
  const [queue, setQueue] = useState<string[]>([]);
  const [originalTotal, setOriginalTotal] = useState(0);
  const [shortcuts, setShortcuts] = useState<Shortcut[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const loaded = useRef(false);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const capabilities = await getBuildCapabilities();
      // Enterprise intentionally has no local Worth Fixing or Ready-to-use
      // projections. Avoid touching their commands altogether.
      if (capabilities.enterpriseManaged) {
        setItems([]);
        setQueue([]);
        setShortcuts([]);
        setOriginalTotal(0);
        setError(null);
        loaded.current = true;
        return;
      }
      const [home, ready] = await Promise.all([
        commands.getHomeWorthFixingSummary(),
        commands.getReadyToUse(null, 50),
      ]);
      if (home.status === "error") {
        setError(home.error);
        return;
      }
      if (ready.status === "error") {
        setError(ready.error);
        return;
      }
      const nextItems = home.data.items.map(itemFrom);
      setItems(nextItems);
      setQueue((current) => {
        const ids = new Set(nextItems.map((item) => item.id));
        const retained = current.filter((id) => ids.has(id));
        const appended = nextItems.map((item) => item.id).filter((id) => !retained.includes(id));
        return [...retained, ...appended];
      });
      setOriginalTotal((current) => Math.max(current, nextItems.length));
      const mapped = await Promise.all(ready.data.items.map(async (artifact) => {
        const bundle = await commands.getReadyArtifactSkillBundle(artifact.artifactId);
        const shortcut = withBundle(shortcutFrom(artifact), bundle.status === "ok" ? bundle.data : undefined);
        if (bundle.status !== "ok" || bundle.data.status !== "ready" || !bundle.data.bundleId) return shortcut;
        const targets = await commands.getSkillBundleInstallTargets(bundle.data.bundleId);
        if (targets.status !== "ok") return shortcut;
        return {
          ...shortcut,
          bundle: {
            ...shortcut.bundle!,
            targets: targets.data,
          },
        };
      }));
      setShortcuts(mapped);
      setError(null);
      loaded.current = true;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Worth fixing could not load.");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
    const onFocus = () => void reload();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [reload]);

  useEffect(() => {
    if (!shortcuts.some((shortcut) => shortcut.bundle?.status === "pending" || shortcut.bundle?.status === "running")) return;
    const timer = window.setInterval(() => void reload(), 3_000);
    return () => window.clearInterval(timer);
  }, [reload, shortcuts]);

  const remove = useCallback((id: string) => {
    setItems((current) => current.filter((item) => item.id !== id));
    setQueue((current) => current.filter((queued) => queued !== id));
  }, []);

  const save = useCallback(async (id: string) => {
    const result = await commands.keepWorthFixingFinding(id);
    if (result.status === "error") {
      setError(result.error);
      return false;
    }
    remove(id);
    setShortcuts((current) => current.some((item) => item.id === result.data.artifact.artifactId)
      ? current
      : [shortcutFrom(result.data.artifact), ...current]);
    return true;
  }, [remove]);

  const dismiss = useCallback(async (id: string, reason: CorrectionReason) => {
    const disposition = dispositionFor(reason);
    const result = reason === "numbers-off"
      ? await commands.correctWorthFixingFinding(id, "The numbers are off.", "home_numbers_off")
      : await commands.dismissWorthFixingFinding(id, disposition);
    if (result.status === "error") {
      setError(result.error);
      return false;
    }
    remove(id);
    return true;
  }, [remove]);

  const defer = useCallback((id: string) => {
    setQueue((current) => current.length < 2 ? current : [...current.filter((queued) => queued !== id), id]);
  }, []);

  const bringToFront = useCallback((id: string) => {
    setQueue((current) => [id, ...current.filter((queued) => queued !== id)]);
  }, []);

  const restore = useCallback(() => setQueue(items.map((item) => item.id)), [items]);

  const copyShortcut = useCallback(async (id: string) => {
    try {
      const shortcut = shortcuts.find((value) => value.id === id);
      if (shortcut?.bundle?.status === "ready" && shortcut.bundle.bundleId) {
        const prompt = await commands.getSkillBundlePrompt(shortcut.bundle.bundleId);
        if (prompt.status === "error") {
          setError(prompt.error);
          return false;
        }
        await navigator.clipboard.writeText(prompt.data);
        return true;
      }
      const detail = await commands.getReadyArtifact(id);
      if (detail.status === "error") {
        setError(detail.error);
        return false;
      }
      await navigator.clipboard.writeText(detail.data.body);
      const action: ReadyArtifactAction = detail.data.card.kind === "runbook"
        ? "share"
        : detail.data.card.kind === "existing_capability"
          ? "show_how"
          : "copy";
      const receipt = await commands.recordReadyArtifactUsed(id, action);
      if (receipt.status === "error") {
        setError(receipt.error);
        return false;
      }
      return true;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "The shortcut could not be copied.");
      return false;
    }
  }, [shortcuts]);

  const buildShortcutSkill = useCallback(async (id: string) => {
    setShortcuts((current) => current.map((shortcut) => shortcut.id === id
      ? { ...shortcut, bundle: { status: "running", stage: "preparing" } }
      : shortcut));
    const result = await commands.buildReadyArtifactSkillBundle(id);
    if (result.status === "error") {
      setShortcuts((current) => current.map((shortcut) => shortcut.id === id
        ? { ...shortcut, bundle: { status: "failed", error: result.error } }
        : shortcut));
      setError(result.error);
      return false;
    }
    setShortcuts((current) => current.map((shortcut) => shortcut.id === id
      // The command returns immediately so the provider process can continue
      // after this screen unmounts. Keep the optimistic running state until
      // the persisted job is visible on the next poll; otherwise a pending
      // response briefly re-enables Build skill and permits a duplicate click.
      ? result.data.status === "pending"
        ? { ...shortcut, bundle: { status: "running" } }
        : withBundle(shortcut, result.data)
      : shortcut));
    return true;
  }, []);

  const installShortcutSkill = useCallback(async (id: string, target: "codex" | "claude" | "pi") => {
    const bundleId = shortcuts.find((shortcut) => shortcut.id === id)?.bundle?.bundleId;
    if (!bundleId) return false;
    const receipt = await commands.installSkillBundle(bundleId, target);
    if (receipt.status === "error") {
      setError(receipt.error);
      return false;
    }
    await reload();
    return true;
  }, [reload, shortcuts]);

  const exportShortcutSkill = useCallback(async (id: string) => {
    const bundleId = shortcuts.find((shortcut) => shortcut.id === id)?.bundle?.bundleId;
    if (!bundleId) return false;
    const receipt = await commands.exportSkillBundle(bundleId);
    if (receipt.status === "error") {
      setError(receipt.error);
      return false;
    }
    await reload();
    return receipt.data;
  }, [reload, shortcuts]);

  const source: HomeSource = {
    items,
    queue,
    originalTotal,
    shortcuts,
    loading: loading && !loaded.current,
    error,
    save,
    dismiss,
    defer,
    bringToFront,
    restore,
    reload,
    copyShortcut,
    buildShortcutSkill,
    installShortcutSkill,
    exportShortcutSkill,
  };
  return <HomeContext.Provider value={source}>{children}</HomeContext.Provider>;
}

export function useHome(): HomeSource {
  const source = useContext(HomeContext);
  if (!source) throw new Error("useHome must be used within a HomeProvider");
  return source;
}

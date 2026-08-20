"use client";

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Check, ChevronDown, Monitor, MousePointerClick, type LucideIcon } from "lucide-react";

import { commands, type OSPermissionsCheck } from "@/lib/utils/tauri";
import { requestPermissionWithFlow } from "@/lib/utils/permission-flow";
import { cn } from "@/lib/utils";

type PermissionStatus = "granted" | "denied" | "checking";
type PermissionKey = "accessibility" | "screenRecording";

const PERMISSION_COPY: Record<PermissionKey, { label: string; description: string; icon: LucideIcon; help: ReactNode[] }> = {
  accessibility: { label: "Accessibility", description: "So Dystil can tell a click from a scroll and spot the steps you repeat.", icon: MousePointerClick, help: [<>Open <kbd className="dystil-permission-kbd">System Settings → Privacy &amp; Security</kbd></>, <>Click <kbd className="dystil-permission-kbd">Accessibility</kbd></>, <>Toggle <b>Dystil</b> on</>] },
  screenRecording: { label: "Screen Recording", description: "So Dystil can capture the work on screen that your organization has enabled.", icon: Monitor, help: [<>Open <kbd className="dystil-permission-kbd">System Settings → Privacy &amp; Security</kbd></>, <>Click <kbd className="dystil-permission-kbd">Screen Recording</kbd></>, <>Toggle <b>Dystil</b> on</>] },
};

function toPermissionStatus(value: OSPermissionsCheck[PermissionKey] | undefined): PermissionStatus {
  if (!value) return "checking";
  return value === "granted" || value === "notNeeded" ? "granted" : "denied";
}

function PermissionRow({ permission, status, isBusy, onGrant }: { permission: PermissionKey; status: PermissionStatus; isBusy: boolean; onGrant: (permission: PermissionKey) => void }) {
  const [showHelp, setShowHelp] = useState(false);
  const { label, description, icon: Icon, help } = PERMISSION_COPY[permission];
  const granted = status === "granted";
  return <article className={cn("dystil-onboarding-item mt-[14px] rounded-2xl border p-[18px] transition", granted ? "border-primary/25 bg-primary/[.025]" : "border-border bg-card")}>
    <div className="flex gap-[14px]"><div className={cn("grid h-[42px] w-[42px] shrink-0 place-items-center rounded-xl", granted ? "bg-primary/10 text-primary" : "bg-muted text-muted-foreground")}><Icon className="h-5 w-5" /></div><div className="min-w-0 flex-1"><div className="flex flex-wrap items-center gap-2.5"><strong className="text-base font-semibold">{label}</strong></div><p className="mt-1.5 text-[13.5px] leading-[1.55] text-muted-foreground">{description}</p>
      {granted ? <span className="mt-3 inline-flex items-center gap-2 text-[13.5px] font-semibold text-primary"><Check className="h-4 w-4" />Granted, thank you</span> : <><div className="mt-[14px] flex flex-wrap gap-2"><button type="button" disabled={isBusy || status === "checking"} onClick={() => onGrant(permission)} className="rounded-[10px] border border-primary bg-card px-[14px] py-[9px] text-[13.5px] font-semibold text-primary transition hover:bg-primary/5 disabled:opacity-50">{isBusy || status === "checking" ? "Requesting…" : "Grant"}</button><button type="button" onClick={() => setShowHelp((open) => !open)} className="inline-flex items-center gap-1 rounded-[10px] border border-border bg-card px-[14px] py-[9px] text-[13.5px] font-semibold text-foreground transition hover:border-primary hover:text-primary">{showHelp ? "Hide steps" : "How do I grant this?"}<ChevronDown className={cn("h-3.5 w-3.5 transition", showHelp && "rotate-180")} /></button></div>{showHelp ? <div className="mt-3 rounded-lg bg-muted/40 px-[14px] py-2"><ol className="ml-4 list-decimal text-[13px] leading-[1.7] text-muted-foreground">{help.map((step, index) => <li key={index}>{step}</li>)}</ol></div> : null}</>}
    </div></div>
  </article>;
}

export function OnboardingPermissionsStep({ onReadyChange, enterpriseManaged }: { onReadyChange: (ready: boolean) => void; enterpriseManaged: boolean }) {
  const [permissions, setPermissions] = useState<OSPermissionsCheck | null>(null);
  const [busyPermission, setBusyPermission] = useState<PermissionKey | null>(null);
  const pendingPermissionRef = useRef<PermissionKey | null>(null);
  const screenRecordingWasMissingRef = useRef(false);
  const checkPermissions = useCallback(async () => { try { const next = await commands.doPermissionsCheck(false); setPermissions(next); return next; } catch (error) { console.error("Failed to check onboarding permissions:", error); return null; } }, []);
  const clearPending = useCallback(() => { pendingPermissionRef.current = null; setBusyPermission(null); }, []);
  useEffect(() => { void checkPermissions(); const interval = setInterval(() => void checkPermissions(), 2000); return () => clearInterval(interval); }, [checkPermissions]);
  const accessibilityStatus = toPermissionStatus(permissions?.accessibility);
  const screenRecordingStatus = toPermissionStatus(permissions?.screenRecording);
  const allGranted = accessibilityStatus === "granted";
  useEffect(() => { onReadyChange(allGranted); return () => onReadyChange(false); }, [allGranted, onReadyChange]);
  useEffect(() => {
    if (!enterpriseManaged) return;
    if (screenRecordingStatus !== "granted") {
      if (screenRecordingStatus === "denied") screenRecordingWasMissingRef.current = true;
      return;
    }
    if (!screenRecordingWasMissingRef.current) return;
    screenRecordingWasMissingRef.current = false;
    void (async () => {
      try {
        await commands.stopCapture();
        await commands.startCapture();
      } catch (error) {
        console.error("Failed to enable screenshot capture after permission was granted:", error);
      }
    })();
  }, [enterpriseManaged, screenRecordingStatus]);
  const handleGrant = async (permission: PermissionKey) => {
    if (pendingPermissionRef.current) return;
    pendingPermissionRef.current = permission;
    setBusyPermission(permission);
    try { await requestPermissionWithFlow(permission); } catch (error) { console.error(`Failed to request ${permission}:`, error); } finally { clearPending(); await checkPermissions(); }
  };
  useEffect(() => { if (!busyPermission) return; const refresh = () => void checkPermissions().then(() => clearPending()); window.addEventListener("focus", refresh); return () => window.removeEventListener("focus", refresh); }, [busyPermission, checkPermissions, clearPending]);
  return <div>
    <PermissionRow permission="accessibility" status={accessibilityStatus} isBusy={busyPermission === "accessibility"} onGrant={handleGrant} />
    {enterpriseManaged ? (
      <PermissionRow permission="screenRecording" status={screenRecordingStatus} isBusy={busyPermission === "screenRecording"} onGrant={handleGrant} />
    ) : (
      <p className="mt-4 text-center text-[12.5px] text-muted-foreground">Screenshot capture is optional and can be enabled later from Settings.</p>
    )}
  </div>;
}

"use client";

import { useEffect, useState } from "react";
import { Check, MousePointerClick } from "lucide-react";
import { commands } from "@/lib/utils/tauri";
import { requestPermissionWithFlow } from "@/lib/utils/permission-flow";

type PermissionName = "accessibility";

export default function PermissionRecoveryPage() {
  const [permissions, setPermissions] = useState<Record<string, string> | null>(null);
  const refresh = async () => {
    try { setPermissions(await commands.doPermissionsCheck(false)); } catch { /* retry on interval */ }
  };
  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 2_000);
    return () => clearInterval(timer);
  }, []);

  const granted = (name: PermissionName) => {
    const value = permissions?.[name];
    return value === "granted" || value === "notNeeded";
  };
  const row = (name: PermissionName, label: string, description: string, icon: React.ReactNode) => (
    <div className="rounded-2xl border border-border bg-card p-5">
      <div className="flex items-center gap-3">
        <span className="grid h-10 w-10 place-items-center rounded-xl bg-muted">{granted(name) ? <Check className="h-5 w-5 text-primary" /> : icon}</span>
        <div><h2 className="font-semibold">{label}</h2><p className="text-sm text-muted-foreground">{description}</p></div>
      </div>
      {!granted(name) && <button className="mt-4 rounded-lg border border-primary px-3 py-2 text-sm font-semibold text-primary" onClick={() => void requestPermissionWithFlow(name).then(refresh)}>Grant</button>}
    </div>
  );

  return <main className="flex min-h-screen items-center justify-center bg-background p-6"><section className="w-full max-w-lg space-y-4"><h1 className="text-2xl font-bold">Dystil needs a quick hand</h1><p className="text-muted-foreground">Turn Accessibility back on and Dystil will resume capture.</p>{row("accessibility", "Accessibility", "Allows Dystil to capture accessibility text and interactions.", <MousePointerClick className="h-5 w-5" />)}</section></main>;
}

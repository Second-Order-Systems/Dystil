"use client";

import { useEffect, useState } from "react";
import { commands, type WorthFixingEvidenceLine } from "@/lib/utils/tauri";

export function FindingEvidence({ findingId }: { findingId: string }) {
  const [items, setItems] = useState<WorthFixingEvidenceLine[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = async () => {
    setError(null);
    const result = await commands.getWorthFixingEvidence(findingId);
    if (result.status === "error") setError(result.error);
    else setItems(result.data);
  };

  useEffect(() => { void load(); }, [findingId]);

  return <section aria-label="What Dystil saw" className="mt-5 border-t border-[#e1dfd9] pt-4">
    {error ? <p className="text-[14px] text-[#8b3028]">Evidence could not be loaded. <button className="font-medium underline underline-offset-2" onClick={() => void load()}>Try again</button></p> : null}
    {!items && !error ? <p className="text-[14px] text-[#68706c]">Loading what Dystil saw…</p> : null}
    {items ? <ul className="grid gap-3">{items.map((item) => <li key={item.evidenceId} className="text-[14px] leading-6 text-[#555d59]"><p>{item.available ? item.description : "This evidence is no longer available"}</p><p className="text-[12px] text-[#7a817d]">{item.app ? `${item.app} · ` : ""}{new Date(item.occurredAt).toLocaleString()}</p></li>)}</ul> : null}
  </section>;
}

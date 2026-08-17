"use client";

import { useSearchParams } from "next/navigation";

import { AskForFix } from "@/components/dystil/pages/ask-for-fix";

/** Not yet redesigned — no written handoff for this screen. */
export default function AskPage() {
  const searchParams = useSearchParams();
  return (
    <div className="px-10 py-8">
      <AskForFix initialText={searchParams.get("initial") ?? ""} />
    </div>
  );
}

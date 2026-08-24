"use client";

import { useSearchParams } from "next/navigation";

import { AskForFix } from "@/components/dystil/pages/ask-for-fix";

export default function NewChatPage() {
  const searchParams = useSearchParams();
  const sessionId = searchParams.get("session");
  return (
    <div className="px-10 py-8">
      <AskForFix
        initialText={searchParams.get("initial") ?? ""}
        sessionId={sessionId ?? undefined}
        readOnly={searchParams.get("view") === "1"}
        fresh={!sessionId}
      />
    </div>
  );
}

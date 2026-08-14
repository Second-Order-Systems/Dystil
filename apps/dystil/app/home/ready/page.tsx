"use client";

import { useRouter } from "next/navigation";
import { ReadyToUse } from "@/components/dystil/pages/ready-to-use";

/**
 * "Your shortcuts" in the new design. Still the pre-redesign component — this
 * screen has no written handoff yet, so it keeps its current composition and
 * inherits the new palette through tokens.
 */
export default function ReadyPage() {
  const router = useRouter();
  return (
    <div className="px-10 py-8">
      <ReadyToUse onAsk={() => router.push("/home/ask")} onWorthFixing={() => router.push("/home")} />
    </div>
  );
}

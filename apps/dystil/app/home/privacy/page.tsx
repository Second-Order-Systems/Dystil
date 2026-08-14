"use client";

import { useRouter } from "next/navigation";
import { Privacy } from "@/components/dystil/pages/privacy";

/** Not yet redesigned — no written handoff for this screen. */
export default function PrivacyPage() {
  const router = useRouter();
  return (
    <div className="px-10 py-8">
      <Privacy onOpenSettings={() => router.push("/home/settings")} />
    </div>
  );
}

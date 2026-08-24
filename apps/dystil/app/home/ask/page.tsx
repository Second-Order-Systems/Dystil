"use client";

import { useEffect } from "react";
import { useRouter, useSearchParams } from "next/navigation";

import { useAppPolicy } from "@/lib/app-policy";

/** Keeps existing Ask links working while conversations live at /home/chat. */
export default function AskPage() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const { policy } = useAppPolicy();
  const initial = searchParams.get("initial");
  useEffect(() => {
    if (!policy) return;
    const target = policy.askBackend === "cloud" ? "/home" : "/home/chat";
    router.replace(initial ? `${target}?initial=${encodeURIComponent(initial)}` : target);
  }, [initial, policy, router]);
  return null;
}

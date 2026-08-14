"use client";

import { YourShortcuts } from "@/components/dystil/home/your-shortcuts";

/**
 * Route kept at /home/ready so existing deep links and the Rust-side
 * `navigate` emit keep resolving; the screen itself is now "Your shortcuts",
 * per the handoff's vocabulary.
 */
export default function ShortcutsPage() {
  return <YourShortcuts />;
}

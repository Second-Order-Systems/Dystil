"use client";

import { Instrument_Sans, Newsreader } from "next/font/google";
import { useEffect } from "react";
import "./globals.css";
import { Providers } from "./providers";

// Both faces are self-hosted at build time by next/font, which the app needs —
// it must work offline, so the design prototype's Google CDN link is not an
// option. Both are variable fonts; omitting `weight` keeps the full axis.
const instrumentSans = Instrument_Sans({
  subsets: ["latin"],
  variable: "--font-instrument-sans",
  display: "swap",
});

const newsreader = Newsreader({
  subsets: ["latin"],
  variable: "--font-newsreader",
  display: "swap",
});

const startupRecoveryScript = `
(function () {
  var recoveryKey = "__dystil_startup_recovery";
  var recoveryCooldownMs = 30000;

  function reloadOnce() {
    try {
      var now = Date.now();
      var lastRecovery = Number(sessionStorage.getItem(recoveryKey) || 0);
      if (now - lastRecovery < recoveryCooldownMs) return;
      sessionStorage.setItem(recoveryKey, String(now));
    } catch (_) {}
    window.location.reload();
  }

  function isStartupChunkFailure(value) {
    var message = "";
    try {
      message = typeof value === "string"
        ? value
        : value && (value.message || value.reason)
          ? String(value.message || value.reason)
          : String(value || "");
    } catch (_) {}
    return /ChunkLoadError|Loading chunk|Unexpected EOF/i.test(message);
  }

  window.addEventListener("error", function (event) {
    if (isStartupChunkFailure(event && (event.error || event.message))) {
      setTimeout(reloadOnce, 0);
    }
  });

  window.addEventListener("unhandledrejection", function (event) {
    if (isStartupChunkFailure(event && event.reason)) {
      setTimeout(reloadOnce, 0);
    }
  });

  setTimeout(function () {
    if (!document.documentElement.hasAttribute("data-dystil-mounted")) {
      reloadOnce();
    }
  }, 8000);
})();
`;

export default function RootLayout({ children }: { children: React.ReactNode }) {
  useEffect(() => {
    const recoverFocus = () => {
      if (document.activeElement === document.body || !document.activeElement) {
        document.body.tabIndex = -1;
        document.body.focus();
      }
    };
    window.addEventListener("focus", recoverFocus);
    return () => window.removeEventListener("focus", recoverFocus);
  }, []);

  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: startupRecoveryScript }} />
      </head>
      <body className={`${instrumentSans.variable} ${newsreader.variable} font-sans scrollbar-hide`}>
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}

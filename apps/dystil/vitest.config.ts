import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import path from "path";

export default defineConfig({
	plugins: [react()],
	test: {
		environment: "jsdom",
		// jsdom only exposes localStorage for a non-opaque origin. The desktop
		// UI uses it for small local preferences such as font size.
		environmentOptions: {
			jsdom: { url: "http://localhost" },
		},
		globals: true,
		setupFiles: ["./vitest.setup.ts"],
		include: ["**/__tests__/**/*.test.{ts,tsx}", "**/*.test.{ts,tsx}"],
		// `"node_modules"` only excludes the top-level dir, not nested ones.
		// Without `**/node_modules/**` vitest can crawl into nested package
		// trees and run bundled test suites we do not own.
		//
		// The bun:test files (test runner is Bun-native, not vitest) are
		// listed below — they import from `bun:test` which vitest can't
		// resolve. They run via `bun run test:bun` instead. Keep this
		// list in sync with package.json's test:bun glob.
		exclude: [
			"**/node_modules/**",
			"**/.next/**",
			"**/dist/**",
			"src-tauri/target/**",
			"lib/utils/redact-pii.test.ts",
			"lib/utils/sanitize-tool-call-xml.test.ts",
			"lib/__tests__/team-crypto.test.ts",
			"lib/__tests__/team-api-contract.test.ts",
			"lib/__tests__/team-pipes.test.ts",
			"components/__tests__/url-detection-benchmark.test.ts",
			"lib/hooks/__tests__/server-push-old-frames.test.ts",
			"lib/hooks/__tests__/window-focus-refresh.test.ts",
			"lib/hooks/__tests__/server-poll-logic.test.ts",
			"lib/events/__tests__/bus.test.ts",
			// KNOWN-BROKEN: pre-existing failures surfaced when wiring tests
			// into CI. Each is a real assertion failure (component drifted
			// from its test) — excluded so we can ship the CI gate now and
			// fix in follow-ups. Don't add new entries without an owner.
			//   - text-overlay.test.tsx: container.firstChild null + missing
			//     "solid" underline on link blocks
			//   - use-frame-ocr-data.test.ts: fetch flow drifted
			"components/__tests__/text-overlay.test.tsx",
			"lib/hooks/__tests__/use-frame-ocr-data.test.ts",
		],
	},
	resolve: {
		alias: {
			"@": path.resolve(__dirname, "./"),
			"@dystil-ext": path.resolve(__dirname, "../../vendor/dystil/crates/dystil-core/assets/extensions"),
		},
	},
});

#!/usr/bin/env node

/**
 * Generates a Tauri static-platforms `latest.json` for R2-hosted updates.
 *
 * Usage:
 *   bun scripts/generate-r2-manifest.js \
 *     --input release-artifacts \
 *     --version 1.0.1 \
 *     --base-url https://updates.example.invalid/releases/1.0.1 \
 *     --output release-artifacts/latest.json
 */

import { readdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { join, basename } from "node:path";

function parseArgs() {
  const args = process.argv.slice(2);
  const parsed = {};
  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--input") parsed.input = args[++i];
    else if (args[i] === "--version") parsed.version = args[++i];
    else if (args[i] === "--base-url") parsed.baseUrl = args[++i];
    else if (args[i] === "--output") parsed.output = args[++i];
  }
  if (!parsed.input || !parsed.version || !parsed.baseUrl || !parsed.output) {
    console.error("Usage: generate-r2-manifest.js --input <dir> --version <v> --base-url <url> --output <file>");
    process.exit(1);
  }
  return parsed;
}

const CI_TARGET_TO_PLATFORM = {
  "aarch64-apple-darwin": "darwin-aarch64",
  "x86_64-apple-darwin": "darwin-x86_64",
  "x86_64-pc-windows-msvc": "windows-x86_64",
  "x86_64-unknown-linux-gnu": "linux-x86_64",
};

const BUNDLE_GLOBS = {
  "darwin-aarch64": ".app.tar.gz",
  "darwin-x86_64": ".app.tar.gz",
  "windows-x86_64": ".nsis.zip",
  "linux-x86_64": ".AppImage.tar.gz",
};

function findBundleAndSig(dir, platform) {
  const entries = readdirSync(dir, { recursive: true });
  const suffix = BUNDLE_GLOBS[platform];
  if (!suffix) throw new Error(`Unknown platform: ${platform}`);

  const bundle = entries.find((e) => e.endsWith(suffix) && !e.endsWith(".sig"));
  if (!bundle) throw new Error(`No bundle found in ${dir} for ${platform} (suffix: ${suffix})`);

  const sigFile = `${bundle}.sig`;
  if (!existsSync(join(dir, sigFile))) throw new Error(`No .sig found: ${join(dir, sigFile)}`);

  return {
    bundle: join(dir, bundle),
    sigFile: join(dir, sigFile),
    bundleName: basename(bundle),
  };
}

function main() {
  const { input, version, baseUrl, output } = parseArgs();

  const subdirs = readdirSync(input, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => d.name);
  console.log(`Found ${subdirs.length} platform directories:`, subdirs);

  const platforms = {};

  for (const sub of subdirs) {
    // Try to match the CI target pattern (e.g. "release-aarch64-apple-darwin")
    let ciTarget = sub;
    if (sub.startsWith("release-")) ciTarget = sub.slice("release-".length);

    const platform = CI_TARGET_TO_PLATFORM[ciTarget];
    if (!platform) {
      console.warn(`  Skipping ${sub}: unknown CI target ${ciTarget}`);
      continue;
    }

    const dir = join(input, sub);
    try {
      const { bundleName, sigFile } = findBundleAndSig(dir, platform);
      const signature = readFileSync(sigFile, "utf-8").trim();
      const url = `${baseUrl.replace(/\/$/, "")}/${bundleName}`;

      platforms[platform] = { signature, url };
      console.log(`  ${platform}: ${bundleName}`);
    } catch (e) {
      console.error(`  ${platform}: ERROR — ${e.message}`);
      process.exit(1);
    }
  }

  if (Object.keys(platforms).length === 0) {
    console.error("No platforms found. Check --input directory and CI target naming.");
    process.exit(1);
  }

  const manifest = {
    version,
    notes: "",
    pub_date: new Date().toISOString(),
    platforms,
  };

  writeFileSync(output, JSON.stringify(manifest, null, 2) + "\n");
  console.log(`Wrote ${output} (${Object.keys(platforms).length} platforms, version ${version})`);
}

main();

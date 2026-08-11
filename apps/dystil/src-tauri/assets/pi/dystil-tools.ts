import type { ExtensionAPI } from "@mariozechner/pi-coding-agent";
import { spawn } from "node:child_process";

const command = process.env.DYSTIL_MCP_COMMAND || "";
const args = JSON.parse(process.env.DYSTIL_MCP_ARGS || "[]") as string[];
// Retrieval is deliberately more generous than an answer call: an explorer
// may need to search, inspect promising records, and then test whether a
// pattern repeats. The 120-second structured-request deadline remains the
// outer bound.
let remainingCalls = 30;

const tools = [
  {
    name: "dystil_get_activity_overview",
    label: "Get activity overview",
    description: "Get a deterministic bounded overview for a time range, including active time, apps, windows, transitions, representative evidence, capture coverage, and empty-state diagnostics. Use first for broad questions.",
    parameters: { type: "object", additionalProperties: false, required: ["start_time", "end_time"], properties: { start_time: { type: "string" }, end_time: { type: "string" }, app_name: { type: "string" }, max_apps: { type: "integer", minimum: 1, maximum: 50 }, max_windows: { type: "integer", minimum: 1, maximum: 60 }, max_snippets: { type: "integer", minimum: 0, maximum: 12 } } },
  },
  {
    name: "dystil_search_activity",
    label: "Search activity",
    description: "FTS5 search over sanitized evidence for exact names, messages, ticket IDs, errors, files, URLs, and quotes. Returns stable evidence IDs and bounded snippets.",
    parameters: { type: "object", additionalProperties: false, required: ["query"], properties: { query: { type: "string" }, start_time: { type: "string" }, end_time: { type: "string" }, source_type: { type: "string", enum: ["frame", "event"] }, app_name: { type: "string" }, window_name: { type: "string" }, browser_url: { type: "string" }, limit: { type: "integer", minimum: 1, maximum: 20 }, offset: { type: "integer", minimum: 0 }, max_snippet_chars: { type: "integer", minimum: 160, maximum: 1200 } } },
  },
  {
    name: "dystil_get_source",
    label: "Get evidence source",
    description: "Get one bounded sanitized evidence record by stable evidence ID after search.",
    parameters: { type: "object", additionalProperties: false, required: ["evidence_id"], properties: { evidence_id: { type: "string" }, max_content_chars: { type: "integer", minimum: 160, maximum: 24000 } } },
  },
  {
    name: "dystil_get_activity_context",
    label: "Get surrounding activity",
    description: "Get chronological sanitized evidence around one search result. Start near 120 seconds and expand only if needed.",
    parameters: { type: "object", additionalProperties: false, required: ["evidence_id"], properties: { evidence_id: { type: "string" }, before_seconds: { type: "integer", minimum: 1, maximum: 3600 }, after_seconds: { type: "integer", minimum: 1, maximum: 3600 }, limit: { type: "integer", minimum: 1, maximum: 50 }, max_content_chars: { type: "integer", minimum: 160, maximum: 8000 } } },
  },
  {
    name: "dystil_get_activity_range",
    label: "Get activity range",
    description: "Read a bounded chronological range of sanitized evidence with source, app, window, URL filters, and pagination.",
    parameters: { type: "object", additionalProperties: false, required: ["start_time", "end_time"], properties: { start_time: { type: "string" }, end_time: { type: "string" }, source_type: { type: "string", enum: ["frame", "event"] }, app_name: { type: "string" }, window_name: { type: "string" }, browser_url: { type: "string" }, limit: { type: "integer", minimum: 1, maximum: 50 }, offset: { type: "integer", minimum: 0 }, max_content_chars: { type: "integer", minimum: 160, maximum: 8000 } } },
  },
] as const;

async function callMcp(tool: string, parameters: unknown, signal: AbortSignal) {
  if (!command) throw new Error("Dystil retrieval sidecar is not configured");
  if (remainingCalls <= 0) throw new Error("Dystil retrieval call budget exhausted");
  remainingCalls -= 1;

  if (signal.aborted) throw new Error("Dystil retrieval was cancelled");
  const initialize = { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-03-26", capabilities: {}, clientInfo: { name: "dystil-pi", version: "1" } } };
  const invoke = { jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: tool, arguments: parameters ?? {} } };
  const input = `${JSON.stringify(initialize)}\n${JSON.stringify(invoke)}\n`;
  const output = await new Promise<{ stdout: string; stderr: string }>((resolve, reject) => {
    const child = spawn(command, args, {
      windowsHide: true,
      env: process.env,
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    let settled = false;
    const finish = (error?: Error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal.removeEventListener("abort", abort);
      if (error) reject(error);
      else resolve({ stdout, stderr });
    };
    const abort = () => {
      child.kill();
      finish(new Error("Dystil retrieval was cancelled"));
    };
    const timer = setTimeout(() => {
      child.kill();
      finish(new Error("Dystil retrieval timed out"));
    }, 15_000);
    signal.addEventListener("abort", abort, { once: true });
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      stdout += chunk;
      if (stdout.length > 512_000) {
        child.kill();
        finish(new Error("Dystil retrieval response was too large"));
      }
    });
    child.stderr.on("data", (chunk: string) => {
      stderr += chunk;
      if (stderr.length > 64_000) stderr = stderr.slice(-64_000);
    });
    child.on("error", (error) => finish(error));
    child.on("close", (code) => {
      if (code === 0) finish();
      else finish(new Error(stderr.trim() || `Dystil retrieval exited with ${code}`));
    });
    child.stdin.end(input);
  });
  const response = output.stdout.split(/\r?\n/).filter(Boolean).map((line) => JSON.parse(line)).find((item) => item.id === 2);
  if (!response) throw new Error(`Dystil retrieval returned no tool response: ${output.stdout.slice(0, 400)} ${output.stderr.slice(0, 200)}`.trim());
  if (response.error) throw new Error(response.error.message || "Dystil retrieval failed");
  return response.result;
}

export default function (pi: ExtensionAPI) {
  for (const tool of tools) {
    pi.registerTool({
      name: tool.name,
      label: tool.label,
      description: tool.description,
      parameters: tool.parameters as any,
      async execute(_toolCallId: string, parameters: unknown, signal: AbortSignal) {
        try {
          const result = await callMcp(tool.name, parameters, signal);
          return { content: Array.isArray(result?.content) ? result.content : [{ type: "text" as const, text: JSON.stringify(result) }] };
        } catch (error) {
          return { content: [{ type: "text" as const, text: `Dystil retrieval error: ${error instanceof Error ? error.message : String(error)}` }], isError: true };
        }
      },
    });
  }
}

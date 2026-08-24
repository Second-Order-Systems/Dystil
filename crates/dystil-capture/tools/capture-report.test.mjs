import { describe, expect, test } from "bun:test";
import {
  aggregateRun,
  duplicateMetrics,
  matchMarkers,
  parseJsonLines,
  renderMarkdown,
} from "./capture-report.mjs";

const common = {
  schema_version: 1,
  run_id: "test-run",
  policy: "baseline",
  measurement_mode: "baseline",
};

function fixture(overrides = {}) {
  return { ...common, sequence: 1, timestamp: "2026-08-10T00:00:00Z", monotonic_ms: 0, ...overrides };
}

function input(overrides = {}) {
  return {
    manifest: {
      schema_version: 1,
      run_id: "test-run",
      policy: "baseline",
      measurement_mode: "baseline",
      remote_writes: false,
      uploads: false,
    },
    events: [],
    captures: [],
    process: [],
    markers: [],
    frames: [],
    dbEvents: [],
    parseErrors: [],
    ...overrides,
  };
}

describe("event schema", () => {
  test("parses valid JSONL and reports the exact invalid line", () => {
    const result = parseJsonLines(`${JSON.stringify(fixture({ kind: "ui_event" }))}\nnot-json\n`, "events");
    expect(result.records).toHaveLength(1);
    expect(result.errors).toEqual([expect.stringContaining("events:2")]);
  });

  test("reports schema mismatches", () => {
    const report = aggregateRun(input({ events: [fixture({ schema_version: 99, kind: "ui_event" })] }));
    expect(report.trust.schema_problems).toHaveLength(1);
  });
});

describe("metric aggregation", () => {
  test("separates a physical click from its enrichment while sharing logical identity", () => {
    const events = [
      fixture({ kind: "ui_event", event_type: "click", source: "physical_click", logical_action_id: "click:1" }),
      fixture({ sequence: 2, kind: "ui_event", event_type: "click", source: "element_enrichment", logical_action_id: "click:1" }),
    ];
    const report = aggregateRun(input({ events }));
    expect(report.events.raw_click_messages).toBe(2);
    expect(report.events.physical_click_messages).toBe(1);
    expect(report.events.element_enrichment_messages).toBe(1);
    expect(report.events.logical_click_identities).toBe(1);
  });
});

describe("duplicate calculations", () => {
  test("distinguishes exact from near consecutive frames", () => {
    const frames = [
      { id: 1, app_name: "Mail", window_name: "Inbox", browser_url: null, content_hash: "10", simhash: "0" },
      { id: 2, app_name: "Mail", window_name: "Inbox", browser_url: null, content_hash: "10", simhash: "0" },
      { id: 3, app_name: "Mail", window_name: "Inbox", browser_url: null, content_hash: "11", simhash: "1" },
      { id: 4, app_name: "Other", window_name: "Inbox", browser_url: null, content_hash: "11", simhash: "1" },
    ];
    expect(duplicateMetrics(frames)).toMatchObject({ exact_consecutive: 1, near_consecutive: 1 });
  });
});

describe("marker matching", () => {
  test("matches expected app and URL within a marker interval", () => {
    const markers = [
      fixture({ kind: "scenario_marker", marker_id: "nav", phase: "start", label: "navigate" }),
      fixture({ sequence: 3, timestamp: "2026-08-10T00:00:05Z", kind: "scenario_marker", marker_id: "nav", phase: "end", label: "navigate", expected_app: "Chrome", expected_url: "example.com" }),
    ];
    const captures = [fixture({ sequence: 2, timestamp: "2026-08-10T00:00:03Z", kind: "accessibility_attempt", outcome: "found", app_name: "chrome.exe", browser_url: "https://example.com/" })];
    expect(matchMarkers(markers, captures)[0]).toMatchObject({ matched: true, candidate_count: 1 });
  });
});

describe("report generation", () => {
  test("renders the key evidence sections", () => {
    const markdown = renderMarkdown(aggregateRun(input()));
    expect(markdown).toContain("## Click accounting");
    expect(markdown).toContain("## Trust and limitations");
    expect(markdown).toContain("remote writes=false");
  });

});

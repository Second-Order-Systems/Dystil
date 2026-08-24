#!/usr/bin/env bun

import { Database } from "bun:sqlite";
import { appendFile, readFile, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join, resolve } from "node:path";

const SCHEMA_VERSION = 1;

export function parseJsonLines(text, source = "jsonl") {
  const records = [];
  const errors = [];
  for (const [index, line] of text.split(/\r?\n/).entries()) {
    if (!line.trim()) continue;
    try {
      records.push(JSON.parse(line));
    } catch (error) {
      errors.push(`${source}:${index + 1}: ${error.message}`);
    }
  }
  return { records, errors };
}

function contextKey(frame) {
  return [frame.app_name ?? "", frame.window_name ?? "", frame.browser_url ?? ""].join("\u001f");
}

export function hammingDistance(left, right) {
  if (left == null || right == null) return null;
  let value = BigInt.asUintN(64, BigInt(left)) ^ BigInt.asUintN(64, BigInt(right));
  let count = 0;
  while (value) {
    value &= value - 1n;
    count += 1;
  }
  return count;
}

export function duplicateMetrics(frames) {
  let exact = 0;
  let near = 0;
  const pairs = [];
  for (let index = 1; index < frames.length; index += 1) {
    const previous = frames[index - 1];
    const current = frames[index];
    if (contextKey(previous) !== contextKey(current)) continue;
    const isExact =
      previous.content_hash != null &&
      current.content_hash != null &&
      previous.content_hash === current.content_hash;
    const distance = hammingDistance(previous.simhash, current.simhash);
    if (isExact) exact += 1;
    else if (distance != null && distance <= 2) near += 1;
    if (isExact || (distance != null && distance <= 2)) {
      pairs.push({ previous_id: previous.id, current_id: current.id, exact: isExact, simhash_distance: distance });
    }
  }
  return { exact_consecutive: exact, near_consecutive: near, pairs };
}

function numberSummary(values) {
  const finite = values.filter(Number.isFinite);
  if (!finite.length) return { count: 0, total: 0, average: null, max: null, p50: null, p95: null, p99: null };
  const sorted = [...finite].sort((a, b) => a - b);
  const percentile = (p) => sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * p) - 1)];
  return {
    count: finite.length,
    total: finite.reduce((sum, value) => sum + value, 0),
    average: finite.reduce((sum, value) => sum + value, 0) / finite.length,
    max: sorted.at(-1),
    p50: percentile(0.5),
    p95: percentile(0.95),
    p99: percentile(0.99),
  };
}

function capturePhaseSummary(records) {
  const values = (key) => records.filter((record) => record[key] != null).map((record) => Number(record[key]));
  const duration = numberSummary(values("duration_us").map((value) => value / 1000));
  const rssBefore = numberSummary(values("rss_before_bytes"));
  const rssAfter = numberSummary(values("rss_after_bytes"));
  const rssDelta = numberSummary(values("rss_delta_bytes"));
  return {
    attempts: records.length,
    duration_ms: duration,
    rss_before_bytes: rssBefore,
    rss_after_bytes: rssAfter,
    rss_delta_bytes: rssDelta,
    node_count: numberSummary(values("node_count")),
    text_bytes: numberSummary(values("text_bytes")),
    truncations: records.filter((record) => record.truncated).length,
    truncation_reasons: countBy(records.filter((record) => record.truncation_reason), (record) => record.truncation_reason),
  };
}

function capturePhaseMetrics(records) {
  const byPhase = {};
  for (const phase of [...new Set(records.map((record) => record.phase).filter(Boolean))].sort()) {
    byPhase[phase] = capturePhaseSummary(records.filter((record) => record.phase === phase));
  }
  const byApp = {};
  for (const app of [...new Set(records.map((record) => record.app_name).filter(Boolean))].sort()) {
    const appPhases = {};
    for (const phase of [...new Set(records.filter((record) => record.app_name === app).map((record) => record.phase).filter(Boolean))].sort()) {
      appPhases[phase] = capturePhaseSummary(records.filter((record) => record.app_name === app && record.phase === phase));
    }
    byApp[app] = appPhases;
  }
  const uia = records.filter((record) => record.phase === "uia_provider_tree");
  const nonUia = records.filter((record) => record.phase && record.phase !== "uia_provider_tree");
  return {
    total: capturePhaseSummary(records),
    by_phase: byPhase,
    by_app: byApp,
    uia_duration_ms: numberSummary(uia.map((record) => Number(record.duration_us) / 1000)),
    non_uia_duration_ms: numberSummary(nonUia.map((record) => Number(record.duration_us) / 1000)),
  };
}

function includesFolded(value, expected) {
  return value?.toLocaleLowerCase().includes(expected.toLocaleLowerCase()) ?? false;
}

function normalizedVisibleFact(value) {
  return String(value ?? "")
    .normalize("NFKC")
    // UIA and PowerShell can disagree on emoji/symbol glyphs (for example a
    // mailbox icon becoming `??`). Ignore formatting, marks, symbols, and
    // punctuation for fixture fact matching while retaining all word tokens.
    .replace(/[\p{Cf}\p{Mn}\p{S}\p{P}]/gu, "")
    .replace(/\s*([,.:;])\s*/g, "$1 ")
    .replace(/\s+/g, " ")
    .trim()
    .toLocaleLowerCase();
}

export function matchMarkers(markers, captures) {
  const attempts = captures.filter((record) => record.kind === "accessibility_attempt" && record.outcome === "found");
  const starts = new Map();
  const matches = [];
  for (const marker of markers.filter((record) => record.kind === "scenario_marker")) {
    if (marker.phase === "start") {
      starts.set(marker.marker_id ?? marker.label, marker);
      continue;
    }
    if (!['end', 'point'].includes(marker.phase)) continue;
    const start = starts.get(marker.marker_id ?? marker.label);
    const startMs = start ? Date.parse(start.timestamp) : Date.parse(marker.timestamp) - 10_000;
    const endMs = Date.parse(marker.timestamp) + 1_000;
    const candidates = attempts.filter((attempt) => {
      const at = Date.parse(attempt.timestamp);
      return at >= startMs && at <= endMs;
    });
    const matched = candidates.find((attempt) =>
      (!marker.expected_app || includesFolded(attempt.app_name, marker.expected_app)) &&
      (!marker.expected_window || includesFolded(attempt.window_title, marker.expected_window)) &&
      (!marker.expected_url || includesFolded(attempt.browser_url, marker.expected_url))
    );
    matches.push({
      marker_id: marker.marker_id ?? marker.label,
      label: marker.label,
      matched: Boolean(matched),
      capture_sequence: matched?.sequence ?? null,
      candidate_count: candidates.length,
      expected_app: marker.expected_app ?? null,
      expected_window: marker.expected_window ?? null,
      expected_url: marker.expected_url ?? null,
    });
  }
  return matches;
}

function eventMatchesFact(event, expected) {
  return event?.event_type === "text" &&
    event?.frame_id != null &&
    normalizedVisibleFact(event.text_content).includes(normalizedVisibleFact(expected));
}

function matchExpectedFacts(expectedFacts, frames, dbEvents) {
  const frameIds = new Set(frames.map((frame) => Number(frame.id)));
  return expectedFacts.map((fact) => {
    const expected = String(fact.text ?? "").trim();
    const normalizedExpected = normalizedVisibleFact(expected);
    const eventOnly = fact.evidence === "linked_event_text";
    const acceptsEvent = eventOnly || fact.evidence === "frame_or_linked_event";
    const frame = eventOnly
      ? null
      : normalizedExpected
        ? frames.find((candidate) => normalizedVisibleFact(candidate.frame_text).includes(normalizedExpected))
        : null;
    const event = acceptsEvent
      ? dbEvents.find((candidate) => frameIds.has(Number(candidate.frame_id)) && eventMatchesFact(candidate, expected))
      : null;
    return {
      label: fact.label ?? null,
      kind: fact.kind ?? null,
      evidence: fact.evidence ?? "frame_text",
      expected,
      matched: Boolean(frame || event),
      frame_id: frame?.id ?? event?.frame_id ?? null,
    };
  });
}

export function aggregateRun({ manifest, events, captures, process, markers, frames, dbEvents, activitySpans = [], expectedFacts = [], parseErrors = [] }) {
  const clickEvents = events.filter((record) => record.kind === "ui_event" && record.event_type === "click");
  const physicalClicks = clickEvents.filter((record) => record.source === "physical_click");
  const enrichments = clickEvents.filter((record) => record.source === "element_enrichment");
  const logicalClicks = new Set(clickEvents.map((record) => record.logical_action_id).filter(Boolean));
  const captureRequests = captures.filter((record) => record.kind === "capture_request");
  const captureResults = captures.filter((record) => record.kind === "capture_result");
  const accessibility = captures.filter((record) => record.kind === "accessibility_attempt");
  const capturePhases = captures.filter((record) => record.kind === "capture_phase");
  const background = captures.filter((record) => record.kind === "background_tree_attempt");
  const persistence = captures.filter((record) => record.kind === "persistence_result");
  const browserFrames = frames.filter((frame) => /(chrome|msedge|edge|firefox)/i.test(frame.app_name ?? ""));
  const duplicates = duplicateMetrics(frames);
  const requestIds = new Set(captureRequests.map((record) => record.capture_id));
  const resultIds = new Set(captureResults.map((record) => record.capture_id));
  const schemaProblems = [...events, ...captures, ...process, ...markers]
    .filter((record) => record.schema_version !== SCHEMA_VERSION)
    .map((record) => `sequence ${record.sequence ?? "unknown"} has schema ${record.schema_version}`);
  const backgroundByReason = Object.fromEntries(
    [...new Set(background.map((record) => record.reason))].sort().map((reason) => [reason, background.filter((record) => record.reason === reason).length]),
  );
  const backgroundByOutcome = Object.fromEntries(
    [...new Set(background.map((record) => record.outcome))].sort().map((outcome) => [outcome, background.filter((record) => record.outcome === outcome).length]),
  );
  const markerMatches = matchMarkers(markers, captures);
  const expectedFactMatches = matchExpectedFacts(expectedFacts, frames, dbEvents);
  const idleRequests = captureRequests.filter((record) => record.heartbeat || record.trigger === "idle");
  const idleResults = captureResults.filter((record) => idleRequests.some((request) => request.capture_id === record.capture_id));
  const dbSizes = process.map((record) => Number(record.database_bytes)).filter(Number.isFinite);
  const persistedResults = captureResults.filter((record) => record.outcome === "persisted").length;
  const messagePump = process.filter((record) => record.kind === "message_pump_sample");

  return {
    schema_version: SCHEMA_VERSION,
    generated_at: new Date().toISOString(),
    run: manifest,
    trust: {
      measurement_kind: manifest.measurement_mode,
      performance_is_valid_ab_measurement: manifest.measurement_mode === "matched_ab",
      remote_writes_declared: manifest.remote_writes,
      uploads_declared: manifest.uploads,
      parse_errors: parseErrors,
      schema_problems: schemaProblems,
      unpaired_capture_requests: [...requestIds].filter((id) => !resultIds.has(id)),
      orphan_capture_results: [...resultIds].filter((id) => !requestIds.has(id)),
      unexplained_frame_delta: frames.length - persistedResults,
      limitations: [
        "Baseline mode measures current behavior; it does not compare a candidate policy yet.",
        "CPU samples include debug instrumentation overhead and become release evidence only in matched A/B runs.",
        "Near-duplicate counts use a SimHash distance of at most two and are candidates for review, not proof of semantic equivalence.",
        "URL coverage is trustworthy only for explicitly marked supported-browser navigations.",
        "Physical-click accuracy requires the fixed ten-click scenario marker; source classification alone is not external ground truth.",
      ],
    },
    events: {
      observed: events.filter((record) => record.kind === "ui_event").length,
      raw_click_messages: clickEvents.length,
      physical_click_messages: physicalClicks.length,
      element_enrichment_messages: enrichments.length,
      logical_click_identities: logicalClicks.size,
      stored_rows: dbEvents.length,
      stored_click_rows: dbEvents.filter((event) => event.event_type === "click").length,
      stored_physical_click_rows: dbEvents.filter((event) => event.event_type === "click" && event.click_count > 0).length,
      stored_enrichment_click_rows: dbEvents.filter((event) => event.event_type === "click" && event.click_count === 0).length,
      linked_rows: dbEvents.filter((event) => event.frame_id != null).length,
    },
    frames: {
      persisted: frames.length,
      text_bytes: frames.reduce((sum, frame) => sum + Number(frame.text_bytes ?? 0), 0),
      exact_consecutive_duplicates: duplicates.exact_consecutive,
      near_consecutive_duplicates: duplicates.near_consecutive,
      duplicate_pairs: duplicates.pairs,
      by_trigger: countBy(frames, (frame) => frame.capture_trigger ?? "unknown"),
    },
    urls: {
      browser_frames: browserFrames.length,
      browser_frames_with_url: browserFrames.filter((frame) => frame.browser_url).length,
      marked_navigation_matches: markerMatches.filter((match) => match.expected_url),
    },
    capture: {
      requests: captureRequests.length,
      results: captureResults.length,
      result_outcomes: countBy(captureResults, (record) => record.outcome),
      duration_ms: numberSummary(captureResults.map((record) => Number(record.duration_ms))),
      accessibility_attempts: accessibility.length,
      accessibility_duration_ms: numberSummary(accessibility.map((record) => Number(record.duration_ms))),
      accessibility_walk_ms: numberSummary(accessibility.map((record) => Number(record.walk_duration_ms))),
      accessibility_node_count: numberSummary(accessibility.map((record) => Number(record.node_count))),
      truncations: accessibility.filter((record) => record.truncated).length,
      timeout_truncations: accessibility.filter((record) => record.truncation_reason === "timeout").length,
      over_budget_walks: accessibility.filter((record) => Number(record.walk_duration_ms) > 250).length,
      background_tree_attempts: background.length,
      background_by_reason: backgroundByReason,
      background_by_outcome: backgroundByOutcome,
      background_duration_ms: numberSummary(background.map((record) => Number(record.duration_ms))),
      persistence_duration_ms: numberSummary(persistence.map((record) => Number(record.duration_ms))),
      normalization_duration_ms: numberSummary(persistence.map((record) => Number(record.normalization_duration_ms))),
      snapshot_bytes: persistence.reduce((sum, record) => sum + Number(record.snapshot_bytes ?? 0), 0),
      phase_metrics: capturePhaseMetrics(capturePhases),
    },
    idle: {
      heartbeat_requests: idleRequests.length,
      outcomes: countBy(idleResults, (record) => record.outcome),
      persisted_frames: idleResults.filter((record) => record.outcome === "persisted").length,
    },
    activity_spans: {
      count: activitySpans.length,
      by_kind: countBy(activitySpans, (span) => span.kind),
      linked_final_frames: activitySpans.filter((span) => span.final_frame_id != null).length,
      total_scroll_delta_x: activitySpans.reduce((sum, span) => sum + Number(span.scroll_delta_x ?? 0), 0),
      total_scroll_delta_y: activitySpans.reduce((sum, span) => sum + Number(span.scroll_delta_y ?? 0), 0),
      spans: activitySpans,
    },
    process: {
      samples: process.filter((record) => record.kind === "process_sample").length,
      cpu_percent: numberSummary(process.filter((record) => record.kind === "process_sample").map((record) => Number(record.cpu_percent))),
      rss_bytes: numberSummary(process.filter((record) => record.kind === "process_sample").map((record) => Number(record.rss_bytes))),
      database_bytes_start: dbSizes.at(0) ?? null,
      database_bytes_end: dbSizes.at(-1) ?? null,
      database_growth_bytes: dbSizes.length ? dbSizes.at(-1) - dbSizes[0] : null,
    },
    message_pump: {
      samples: messagePump.length,
      duration_ms: numberSummary(messagePump.map((record) => Number(record.duration_us) / 1000)),
      stalls_over_20_ms: messagePump.filter((record) => Number(record.duration_us) > 20_000).length,
      stalls_over_100_ms: messagePump.filter((record) => Number(record.duration_us) > 100_000).length,
    },
    markers: {
      count: markers.filter((record) => record.kind === "scenario_marker").length,
      matches: markerMatches,
      unmatched: markerMatches.filter((match) => !match.matched).length,
    },
    expected_facts: {
      count: expectedFactMatches.length,
      matches: expectedFactMatches,
      unmatched: expectedFactMatches.filter((fact) => !fact.matched).length,
    },
  };
}

function countBy(values, key) {
  const counts = {};
  for (const value of values) {
    const name = String(key(value) ?? "unknown");
    counts[name] = (counts[name] ?? 0) + 1;
  }
  return Object.fromEntries(Object.entries(counts).sort(([left], [right]) => left.localeCompare(right)));
}

function formatBytes(value) {
  if (value == null) return "not available";
  if (value < 1024) return `${value} B`;
  if (value < 1024 ** 2) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / 1024 ** 2).toFixed(1)} MiB`;
}

export function renderMarkdown(report) {
  const integrity = report.trust.parse_errors.length + report.trust.schema_problems.length + report.trust.unpaired_capture_requests.length + report.trust.orphan_capture_results.length + Math.abs(report.trust.unexplained_frame_delta);
  const phaseRows = Object.entries(report.capture.phase_metrics.by_phase).map(([phase, summary]) =>
    `| ${phase} | ${summary.attempts} | ${summary.duration_ms.total?.toFixed(2) ?? "n/a"} | ${summary.duration_ms.p50?.toFixed(2) ?? "n/a"} | ${summary.rss_after_bytes.max == null ? "n/a" : formatBytes(summary.rss_after_bytes.max)} |`,
  );
  const appPhaseRows = Object.entries(report.capture.phase_metrics.by_app).flatMap(([app, phases]) =>
    Object.entries(phases).map(([phase, summary]) =>
      `| ${app} | ${phase} | ${summary.attempts} | ${summary.duration_ms.total?.toFixed(2) ?? "n/a"} | ${summary.duration_ms.p50?.toFixed(2) ?? "n/a"} |`,
    ),
  );
  const lines = [
    "# Dystil capture baseline report",
    "",
    `Generated: ${report.generated_at}`,
    "",
    "## Verdict",
    "",
    `This is a **${report.run.measurement_mode}** run of policy **${report.run.policy}**. ${report.trust.performance_is_valid_ab_measurement ? "Performance is from a matched A/B run." : "CPU/RAM values are diagnostic baseline data, not a candidate comparison."}`,
    "",
    `Instrumentation integrity issues: **${integrity}**. Unmatched scenario states: **${report.markers.unmatched}**.`,
    `Unmatched expected visible facts: **${report.expected_facts.unmatched}/${report.expected_facts.count}**.`,
    "",
    "## Capture and noise",
    "",
    "| Metric | Value |",
    "|---|---:|",
    `| Persisted frames | ${report.frames.persisted} |`,
    `| Frame text | ${formatBytes(report.frames.text_bytes)} |`,
    `| Exact consecutive duplicates | ${report.frames.exact_consecutive_duplicates} |`,
    `| Near consecutive duplicates | ${report.frames.near_consecutive_duplicates} |`,
    `| Capture requests/results | ${report.capture.requests}/${report.capture.results} |`,
    `| Real accessibility attempts | ${report.capture.accessibility_attempts} |`,
    `| Total UIA walk time | ${report.capture.accessibility_walk_ms.total?.toFixed(2) ?? "n/a"} ms |`,
    `| UIA walks over 250 ms | ${report.capture.over_budget_walks} |`,
    `| Background tree attempts | ${report.capture.background_tree_attempts} |`,
    `| Background periodic attempts | ${report.capture.background_by_reason.periodic ?? 0} |`,
    `| Background focus attempts | ${report.capture.background_by_reason.focus ?? 0} |`,
    `| Compact activity spans | ${report.activity_spans.count} |`,
    `| Spans linked to final frame | ${report.activity_spans.linked_final_frames} |`,
    `| Accessibility truncations | ${report.capture.truncations} |`,
    `| UIA phase time | ${report.capture.phase_metrics.uia_duration_ms.total?.toFixed(2) ?? "n/a"} ms |`,
    `| Non-UIA phase time | ${report.capture.phase_metrics.non_uia_duration_ms.total?.toFixed(2) ?? "n/a"} ms |`,
    "",
    "## Phase attribution",
    "",
    "| Phase | Attempts | Total ms | P50 ms | Peak RSS after phase |",
    "|---|---:|---:|---:|---:|",
    ...(phaseRows.length ? phaseRows : ["| No phase diagnostics | 0 | n/a | n/a | n/a |"]),
    "",
    "## Phase attribution by application",
    "",
    "| Application | Phase | Attempts | Total ms | P50 ms |",
    "|---|---|---:|---:|---:|",
    ...(appPhaseRows.length ? appPhaseRows : ["| No app phase diagnostics | | 0 | n/a | n/a |"]),
    "",
    "## Click accounting",
    "",
    "| Metric | Value |",
    "|---|---:|",
    `| Physical click messages | ${report.events.physical_click_messages} |`,
    `| Element-enrichment click messages | ${report.events.element_enrichment_messages} |`,
    `| Raw click messages | ${report.events.raw_click_messages} |`,
    `| Logical click identities | ${report.events.logical_click_identities} |`,
    `| Stored click rows | ${report.events.stored_click_rows} |`,
    `| Stored physical/enrichment rows | ${report.events.stored_physical_click_rows}/${report.events.stored_enrichment_click_rows} |`,
    "",
    "## Idle and URLs",
    "",
    `Heartbeat requests: **${report.idle.heartbeat_requests}**; heartbeat-persisted frames: **${report.idle.persisted_frames}**.`,
    "",
    `Browser frames with URL: **${report.urls.browser_frames_with_url}/${report.urls.browser_frames}**. Marked URL transitions are the authoritative coverage check.`,
    "",
    "## Resources",
    "",
    `CPU average/max: **${report.process.cpu_percent.average?.toFixed(2) ?? "n/a"}% / ${report.process.cpu_percent.max?.toFixed(2) ?? "n/a"}%**.`,
    "",
    `RSS peak: **${formatBytes(report.process.rss_bytes.max)}**. Database growth: **${formatBytes(report.process.database_growth_bytes)}**.`,
    "",
    `Foreground message-pump p50/p95/p99: **${report.message_pump.duration_ms.p50?.toFixed(2) ?? "n/a"}/${report.message_pump.duration_ms.p95?.toFixed(2) ?? "n/a"}/${report.message_pump.duration_ms.p99?.toFixed(2) ?? "n/a"} ms**; stalls >20 ms/>100 ms: **${report.message_pump.stalls_over_20_ms}/${report.message_pump.stalls_over_100_ms}**.`,
    "",
    "## Background tree reasons",
    "",
    "```json",
    JSON.stringify(report.capture.background_by_reason, null, 2),
    "```",
    "",
    "## Scenario markers",
    "",
    ...(report.markers.matches.length
      ? report.markers.matches.map((match) => `- ${match.matched ? "PASS" : "FAIL"}: ${match.label} (${match.candidate_count} candidate captures)`)
      : ["No manual scenario markers were recorded."]),
    "",
    "## Trust and limitations",
    "",
    ...report.trust.limitations.map((value) => `- ${value}`),
    ...(report.trust.parse_errors.length ? ["", "Parse errors:", ...report.trust.parse_errors.map((value) => `- ${value}`)] : []),
    "",
    "## Locality declaration",
    "",
    `The harness manifest declares remote writes=${report.run.remote_writes} and uploads=${report.run.uploads}. The standalone harness contains no sync engine wiring.`,
    "",
  ];
  return lines.join("\n");
}

async function readJson(path) {
  return JSON.parse(await readFile(path, "utf8"));
}

async function readJsonl(path) {
  try {
    return parseJsonLines(await readFile(path, "utf8"), path);
  } catch (error) {
    if (error.code === "ENOENT") return { records: [], errors: [`${path}: missing`] };
    throw error;
  }
}

async function readExpectedFacts(runDir) {
  try {
    const value = await readJson(join(runDir, "expected-facts.json"));
    return Array.isArray(value.facts) ? value.facts : [];
  } catch (error) {
    if (error.code === "ENOENT") return [];
    throw error;
  }
}

function readDatabase(runDir, manifest) {
  const database = new Database(join(runDir, "db.sqlite"), { readonly: true });
  const frames = database.query(`
    SELECT id, timestamp, app_name, window_name, browser_url, document_path,
           capture_trigger, LENGTH(COALESCE(frame_text, '')) AS text_bytes,
           CAST(content_hash AS TEXT) AS content_hash,
           CAST(simhash AS TEXT) AS simhash, frame_text,
           ax_capture_diagnostics_json
    FROM frames WHERE id > ? ORDER BY id
  `).all(manifest.baseline_frame_id ?? 0);
  const dbEvents = database.query(`
    SELECT id, timestamp, event_type, click_count, app_name, window_title,
           browser_url, x, y, element_role, element_name, element_value, text_content, frame_id
    FROM ui_events WHERE id > ? ORDER BY id
  `).all(manifest.baseline_event_id ?? 0);
  const hasActivitySpans = database
    .query("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'activity_spans'")
    .get();
  const activitySpans = hasActivitySpans
    ? database.query(`
        SELECT id, kind, started_at, ended_at, duration_ms, event_count, app_sequence_json,
               final_app_name, final_window_title, final_target_json,
               scroll_delta_x, scroll_delta_y, final_frame_id
        FROM activity_spans ORDER BY id
      `).all()
    : [];
  database.close();
  return { frames, dbEvents, activitySpans };
}

export async function analyzeRun(runDirectory) {
  const runDir = resolve(runDirectory);
  const manifest = await readJson(join(runDir, "run.json"));
  const sources = await Promise.all(
    ["events.jsonl", "captures.jsonl", "process.jsonl", "markers.jsonl"].map((name) => readJsonl(join(runDir, name))),
  );
  const [events, captures, process, markers] = sources.map((source) => source.records);
  const parseErrors = sources.flatMap((source) => source.errors);
  const { frames, dbEvents, activitySpans } = readDatabase(runDir, manifest);
  const expectedFacts = await readExpectedFacts(runDir);
  const report = aggregateRun({ manifest, events, captures, process, markers, frames, dbEvents, activitySpans, expectedFacts, parseErrors });
  await writeFile(join(runDir, "comparison.json"), `${JSON.stringify(report, null, 2)}\n`);
  await writeFile(join(runDir, "comparison.md"), renderMarkdown(report));
  return report;
}

function parseOptions(args) {
  const options = {};
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (!argument.startsWith("--")) throw new Error(`unexpected argument: ${argument}`);
    const name = argument.slice(2).replaceAll("-", "_");
    const value = args[index + 1];
    if (!value || value.startsWith("--")) throw new Error(`${argument} requires a value`);
    options[name] = value;
    index += 1;
  }
  return options;
}

async function appendMarker(options) {
  if (!options.run_dir || !options.label || !options.phase) {
    throw new Error("mark requires --run-dir, --label, and --phase start|end|point");
  }
  if (!['start', 'end', 'point'].includes(options.phase)) throw new Error("invalid marker phase");
  const manifest = await readJson(join(resolve(options.run_dir), "run.json"));
  const marker = {
    schema_version: SCHEMA_VERSION,
    run_id: manifest.run_id,
    policy: manifest.policy,
    measurement_mode: manifest.measurement_mode,
    sequence: Date.now() * 1000 + Math.floor(Math.random() * 1000),
    timestamp: new Date().toISOString(),
    monotonic_ms: null,
    kind: "scenario_marker",
    marker_id: options.marker_id ?? options.label,
    phase: options.phase,
    label: options.label,
    expected_app: options.expected_app ?? null,
    expected_window: options.expected_window ?? null,
    expected_url: options.expected_url ?? null,
    notes: options.notes ?? null,
  };
  await appendFile(join(resolve(options.run_dir), "markers.jsonl"), `${JSON.stringify(marker)}\n`);
  return marker;
}

async function main() {
  const [command, ...args] = process.argv.slice(2);
  const options = parseOptions(args);
  if (command === "analyze") {
    if (!options.run_dir) throw new Error("analyze requires --run-dir");
    const report = await analyzeRun(options.run_dir);
    console.log(`Wrote ${resolve(options.run_dir, "comparison.json")}`);
    console.log(`Wrote ${resolve(options.run_dir, "comparison.md")}`);
    console.log(`Frames=${report.frames.persisted}, raw clicks=${report.events.raw_click_messages}, background trees=${report.capture.background_tree_attempts}`);
  } else if (command === "mark") {
    const marker = await appendMarker(options);
    console.log(`Recorded ${marker.phase} marker: ${marker.label}`);
  } else {
    throw new Error("usage: bun capture-report.mjs analyze --run-dir <path> | mark --run-dir <path> --phase <start|end|point> --label <label> [--expected-app value] [--expected-window value] [--expected-url value]");
  }
}

if (import.meta.main) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}

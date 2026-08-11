//! App lifecycle adapter for the local Worth Fixing engine.
//!
//! Business rules live in `dystil-insights`; this module only connects the
//! capture database, selected AI runtime, settings, and periodic Tauri task.

use chrono::{DateTime, FixedOffset, Timelike, Utc};
use dystil_insights::{
    capture_cursor, cleanup_diagnostics, commit_compaction_checkpoint,
    compact_activity_incremental, enhanced_diagnostics_enabled, known_source_refs,
    last_successful_steward_wake_at, load_compaction_state, mark_source_deleted,
    pending_explorer_batch_id, pending_observation_stats, record_wake_start,
    recoverable_explorer_job, recoverable_job, resolve_capture_evidence, run_explorer_batch,
    run_explorer_batch_with_compaction, run_steward_wake, upsert_evidence, wake_reason_started,
    CaptureAdmissionRules, CompactionConfig, DiagnosticRetention, EvidenceRecord, SourceActivity,
    WakePolicy, WakeReason, WakeState,
};
use sqlx::{Row, SqlitePool};
use tauri::{AppHandle, Manager};
use tracing::{info, warn};

const SOURCE_NAMESPACE: &str = "local-capture";
// Fetch a look-ahead page from each source, then consume one merged,
// timestamp-ordered batch. Advancing the two cursors independently would
// eventually drain one source and destroy the chronology of the other.
const LOOK_AHEAD_PER_SOURCE: i64 = 200;
const MERGED_BATCH_LIMIT: usize = 200;

pub(crate) fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        loop {
            if let Err(error) = tick(&app).await {
                warn!(%error, "Worth Fixing background tick postponed");
            }
            tokio::time::sleep(std::time::Duration::from_secs(15 * 60)).await;
        }
    });
}

fn admission_rules(app: &AppHandle) -> CaptureAdmissionRules {
    let settings = crate::store::SettingsStore::get(app)
        .ok()
        .flatten()
        .unwrap_or_default();
    CaptureAdmissionRules {
        excluded_apps: Vec::new(),
        excluded_windows: settings.recording.ignored_windows,
        excluded_urls: settings.recording.ignored_urls,
        ignore_private_windows: settings.recording.ignore_incognito_windows,
    }
}

async fn capture_pool(app: &AppHandle) -> Option<SqlitePool> {
    let state = app.state::<crate::recording::RecordingState>();
    let server = state.server.lock().await;
    server.as_ref().map(|server| server.db.pool.clone())
}

async fn reconcile_live_policy(
    insights: &SqlitePool,
    capture: &SqlitePool,
    rules: &CaptureAdmissionRules,
) -> Result<(), String> {
    for (namespace, source_id) in known_source_refs(insights, 10_000)
        .await
        .map_err(|e| e.to_string())?
    {
        if namespace != SOURCE_NAMESPACE {
            continue;
        }
        match resolve_capture_evidence(capture, &namespace, &source_id, rules)
            .await
            .map_err(|e| e.to_string())?
        {
            Some(record) => upsert_evidence(insights, &record)
                .await
                .map_err(|e| e.to_string())?,
            None => {
                mark_source_deleted(insights, &namespace, &source_id)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

async fn next_source_records(
    insights: &SqlitePool,
    capture: &SqlitePool,
    rules: &CaptureAdmissionRules,
) -> Result<(Vec<EvidenceRecord>, i64, i64), String> {
    let frame_cursor = capture_cursor(insights, "frames")
        .await
        .map_err(|e| e.to_string())?;
    let event_cursor = capture_cursor(insights, "events")
        .await
        .map_err(|e| e.to_string())?;
    let frames = sqlx::query("SELECT id FROM frames WHERE id>?1 ORDER BY id LIMIT ?2")
        .bind(frame_cursor)
        .bind(LOOK_AHEAD_PER_SOURCE)
        .fetch_all(capture)
        .await
        .map_err(|e| e.to_string())?;
    let mut candidates = Vec::new();
    for row in frames {
        let id = row.get::<i64, _>("id");
        let record =
            resolve_capture_evidence(capture, SOURCE_NAMESPACE, &format!("frame:{id}"), rules)
                .await
                .map_err(|e| e.to_string())?;
        if let Some(record) = record {
            candidates.push(("frames", id, record));
        }
    }
    let events = sqlx::query("SELECT id FROM ui_events WHERE id>?1 ORDER BY id LIMIT ?2")
        .bind(event_cursor)
        .bind(LOOK_AHEAD_PER_SOURCE)
        .fetch_all(capture)
        .await
        .map_err(|e| e.to_string())?;
    for row in events {
        let id = row.get::<i64, _>("id");
        let record =
            resolve_capture_evidence(capture, SOURCE_NAMESPACE, &format!("event:{id}"), rules)
                .await
                .map_err(|e| e.to_string())?;
        if let Some(record) = record {
            candidates.push(("events", id, record));
        }
    }
    candidates.sort_by(|a, b| {
        a.2.occurred_at
            .cmp(&b.2.occurred_at)
            .then(a.2.evidence_id.cmp(&b.2.evidence_id))
    });
    candidates.truncate(MERGED_BATCH_LIMIT);
    let last_frame = candidates
        .iter()
        .filter(|(source, _, _)| *source == "frames")
        .map(|(_, id, _)| *id)
        .max()
        .unwrap_or(frame_cursor);
    let last_event = candidates
        .iter()
        .filter(|(source, _, _)| *source == "events")
        .map(|(_, id, _)| *id)
        .max()
        .unwrap_or(event_cursor);
    let records = candidates
        .into_iter()
        .map(|(_, _, record)| record)
        .filter(|record| record.policy_allowed && !record.sensitive)
        .collect();
    Ok((records, last_frame, last_event))
}

async fn maybe_explore(
    app: &AppHandle,
    insights: &SqlitePool,
    capture: &SqlitePool,
    timezone: &str,
) -> Result<(), String> {
    let recording = app.state::<crate::recording::RecordingState>();
    if let Some(batch_id) = pending_explorer_batch_id(insights)
        .await
        .map_err(|e| e.to_string())?
    {
        let runtime = crate::ai_runtime::resolve(app, &recording, capture, timezone)
            .await
            .map_err(|e| e.to_string())?;
        run_explorer_batch(insights, runtime.as_ref(), &batch_id, timezone, &[])
            .await
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    let rules = admission_rules(app);
    let (records, last_frame, last_event) = next_source_records(insights, capture, &rules).await?;
    let prior_frame = capture_cursor(insights, "frames")
        .await
        .map_err(|e| e.to_string())?;
    let prior_event = capture_cursor(insights, "events")
        .await
        .map_err(|e| e.to_string())?;
    if last_frame == prior_frame && last_event == prior_event {
        return Ok(());
    }
    let mut state = load_compaction_state(insights)
        .await
        .map_err(|e| e.to_string())?;
    let source = records
        .iter()
        .filter_map(|record| {
            Some(SourceActivity {
                evidence_id: record.evidence_id.clone(),
                occurred_at: DateTime::parse_from_rfc3339(&record.occurred_at)
                    .ok()?
                    .with_timezone(&Utc),
                app: record.app.clone(),
                window: record.window.clone(),
                url: None,
                text: record.excerpt.clone(),
                content_hash: None,
            })
        })
        .collect::<Vec<_>>();
    let compact = compact_activity_incremental(&source, CompactionConfig::default(), &mut state);
    if compact.is_empty() {
        commit_compaction_checkpoint(insights, &state, last_frame, last_event)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    let runtime = crate::ai_runtime::resolve(app, &recording, capture, timezone)
        .await
        .map_err(|e| e.to_string())?;
    let batch_id = format!(
        "capture-f{}-{}-e{}-{}",
        prior_frame + 1,
        last_frame,
        prior_event + 1,
        last_event
    );
    let result = run_explorer_batch_with_compaction(
        insights,
        runtime.as_ref(),
        &batch_id,
        timezone,
        &records,
        &compact,
    )
    .await;
    if result.is_ok()
        || recoverable_explorer_job(insights, &batch_id)
            .await
            .map_err(|e| e.to_string())?
            .is_some()
    {
        commit_compaction_checkpoint(insights, &state, last_frame, last_event)
            .await
            .map_err(|e| e.to_string())?;
    }
    result.map(|_| ()).map_err(|e| e.to_string())
}

async fn maybe_steward(
    app: &AppHandle,
    insights: &SqlitePool,
    capture: &SqlitePool,
    timezone: &str,
) -> Result<(), String> {
    let pending = pending_observation_stats(insights)
        .await
        .map_err(|e| e.to_string())?;
    let recovery = recoverable_job(insights)
        .await
        .map_err(|e| e.to_string())?
        .is_some();
    if pending.count == 0 && !recovery {
        return Ok(());
    }
    let offset = timezone
        .parse::<FixedOffset>()
        .unwrap_or_else(|_| FixedOffset::east_opt(0).unwrap());
    let local_now = Utc::now().with_timezone(&offset);
    let local_day = local_now.date_naive();
    let now = Utc::now();
    let oldest_pending_minutes = pending
        .oldest_admitted_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| {
            now.signed_duration_since(value.with_timezone(&Utc))
                .num_minutes()
                .max(0)
        })
        .unwrap_or(0);
    let minutes_since_last_successful_wake = last_successful_steward_wake_at(insights)
        .await
        .map_err(|e| e.to_string())?
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| {
            now.signed_duration_since(value.with_timezone(&Utc))
                .num_minutes()
                .max(0)
        })
        .unwrap_or(oldest_pending_minutes);
    let provider_ready = crate::ai_presets::active(capture)
        .await
        .map_err(|e| e.to_string())?
        .is_some();
    let resource_permitted = crate::store::SettingsStore::get(app)
        .ok()
        .flatten()
        .and_then(|settings| settings.recording.power_mode)
        .as_deref()
        != Some("battery_saver");
    let end_of_day = local_now.hour() >= 20
        && !wake_reason_started(insights, &local_day.to_string(), "end_of_day")
            .await
            .map_err(|e| e.to_string())?;
    let state = WakeState {
        pending_observations: pending.count,
        pending_episode_groups: pending.episode_groups,
        minutes_since_last_successful_wake,
        oldest_pending_minutes,
        job_running: false,
        explicit_request: false,
        end_of_active_day: end_of_day,
        recovery_pending: recovery,
        provider_ready,
        resource_permitted,
    };
    let Some(reason) = WakePolicy::default().decide(&state) else {
        return Ok(());
    };
    let runtime = crate::ai_runtime::resolve(
        app,
        &app.state::<crate::recording::RecordingState>(),
        capture,
        timezone,
    )
    .await
    .map_err(|e| e.to_string())?;
    let reason_name = match reason {
        WakeReason::ObservationVolume => "observation_volume",
        WakeReason::ObservationBurst => "observation_burst",
        WakeReason::PendingDeadline => "pending_deadline",
        WakeReason::ExplicitRequest => "explicit_request",
        WakeReason::EndOfDay => "end_of_day",
        WakeReason::Recovery => "recovery",
    };
    record_wake_start(
        insights,
        &local_day.to_string(),
        reason_name,
        reason != WakeReason::Recovery,
    )
    .await
    .map_err(|e| e.to_string())?;
    run_steward_wake(
        insights,
        runtime.as_ref(),
        &local_day.to_string(),
        timezone,
        reason_name,
        250,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn tick(app: &AppHandle) -> Result<(), String> {
    let Some(capture) = capture_pool(app).await else {
        return Ok(());
    };
    let state = app.state::<crate::worth_fixing_commands::WorthFixingState>();
    let insights = state.pool(app).await?;
    let diagnostics = crate::log_files::get_data_dir(app)
        .map_err(|error| error.to_string())?
        .join("worth-fixing-diagnostics");
    std::fs::create_dir_all(&diagnostics).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&diagnostics, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    let retention = if enhanced_diagnostics_enabled(insights)
        .await
        .map_err(|error| error.to_string())?
    {
        DiagnosticRetention::enhanced()
    } else {
        DiagnosticRetention::default()
    };
    cleanup_diagnostics(&diagnostics, std::time::SystemTime::now(), retention)
        .map_err(|error| error.to_string())?;
    let rules = admission_rules(app);
    reconcile_live_policy(insights, &capture, &rules).await?;
    let timezone = crate::ai::local_timezone_offset();
    if let Err(error) = maybe_explore(app, insights, &capture, &timezone).await {
        warn!(%error, "Worth Fixing Explorer postponed");
    }
    maybe_steward(app, insights, &capture, &timezone).await?;
    info!("Worth Fixing background tick completed");
    Ok(())
}

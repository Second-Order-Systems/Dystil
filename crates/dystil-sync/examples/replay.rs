use clap::Parser;
use std::path::PathBuf;

use dystil_sync::replay_sync::{run_replay, ReplayConfig};

#[derive(Parser)]
#[command(name = "dystil-replay")]
#[command(about = "Replay the Dystil sync pipeline against a dystil SQLite database")]
struct Args {
    #[arg(short, long, default_value = "~/.dystil/db.sqlite")]
    db: PathBuf,

    #[arg(long, default_value = "replay.html")]
    output: PathBuf,

    #[arg(long, default_value_t = 120)]
    sync_interval_secs: u64,

    #[arg(long, default_value_t = 15)]
    screen_settle_lag_secs: u64,

    #[arg(long, default_value_t = 7)]
    cold_start_lookback_days: u64,

    #[arg(long, default_value_t = 300)]
    segment_inactivity_secs: i64,

    #[arg(long, default_value_t = 900)]
    segment_max_duration_secs: i64,

    #[arg(long, default_value_t = 10000)]
    segment_max_tokens: u32,

    #[arg(long, default_value_t = 20)]
    screen_dedupe_seconds: i64,

    #[arg(long, default_value = "replay-device")]
    machine_id: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let db_path = {
        let raw = args.db.to_string_lossy().to_string();
        if raw.starts_with("~/") {
            if let Ok(home) = std::env::var("HOME") {
                PathBuf::from(home).join(&raw[2..])
            } else {
                args.db.clone()
            }
        } else {
            args.db.clone()
        }
    };

    if !db_path.exists() {
        eprintln!("error: database not found at {}", db_path.display());
        std::process::exit(1);
    }

    let config = ReplayConfig {
        db_path: db_path.display().to_string(),
        sync_interval_secs: args.sync_interval_secs,
        screen_settle_lag_secs: args.screen_settle_lag_secs,
        cold_start_lookback_days: args.cold_start_lookback_days,
        segment_inactivity_secs: args.segment_inactivity_secs,
        segment_max_duration_secs: args.segment_max_duration_secs,
        segment_max_tokens: args.segment_max_tokens,
        screen_dedupe_seconds: args.screen_dedupe_seconds,
    };

    eprintln!("reading from {} ...", db_path.display());

    let data = run_replay(&db_path, &args.machine_id, &config)
        .await
        .unwrap_or_else(|err| {
            eprintln!("error: {err}");
            std::process::exit(1);
        });

    eprintln!(
        "events: {} ({} kept, {} dropped) in {} segments across {} iterations",
        data.summary.total_events_read,
        data.summary.total_kept,
        data.summary.total_dropped,
        data.summary.total_segments,
        data.summary.total_iterations,
    );
    eprintln!(
        "data range: {} → {}",
        data.summary.data_start, data.summary.data_end,
    );

    let json = serde_json::to_string_pretty(&data).unwrap_or_else(|err| {
        eprintln!("error serializing replay data: {err}");
        std::process::exit(1);
    });

    let html_template = include_str!("replay.html");
    let html = html_template.replace(
        "/* REPLAY_DATA_PLACEHOLDER */",
        &format!("var REPLAY_DATA = {};", json),
    );

    std::fs::write(&args.output, html).unwrap_or_else(|err| {
        eprintln!("error writing {}: {err}", args.output.display());
        std::process::exit(1);
    });

    eprintln!("wrote {}", args.output.display());
}

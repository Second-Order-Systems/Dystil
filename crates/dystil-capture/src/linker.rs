use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

const LINKER_CHANNEL_BUFFER: usize = 1_024;
const LINKER_TTL: Duration = Duration::from_secs(60);
const LINKER_CAPACITY: usize = 4_096;

#[derive(Debug, Clone, Copy)]
pub enum DystilLinkDropReason {
    Drm,
    Paused,
    Lagged,
    CaptureError,
    Other,
}

#[derive(Debug)]
enum LinkerMessage {
    EventPersisted {
        correlation_id: u64,
        row_id: i64,
    },
    FrameCaptured {
        frame_id: i64,
        correlation_ids: Vec<u64>,
    },
    TriggerDropped {
        correlation_ids: Vec<u64>,
        reason: DystilLinkDropReason,
    },
}

#[derive(Clone)]
pub struct DystilLinkerSender {
    sender: mpsc::Sender<LinkerMessage>,
    next_id: Arc<AtomicU64>,
}

impl DystilLinkerSender {
    pub fn frame_captured(&self, frame_id: i64, correlation_ids: Vec<u64>) {
        let _ = self.sender.try_send(LinkerMessage::FrameCaptured {
            frame_id,
            correlation_ids,
        });
    }

    pub fn trigger_dropped(&self, correlation_ids: Vec<u64>, reason: DystilLinkDropReason) {
        let _ = self.sender.try_send(LinkerMessage::TriggerDropped {
            correlation_ids,
            reason,
        });
    }

    pub fn next_correlation_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn event_persisted(&self, correlation_id: u64, row_id: i64) {
        let _ = self.sender.try_send(LinkerMessage::EventPersisted {
            correlation_id,
            row_id,
        });
    }
}

/// Dystil-owned actor pairing persisted UI events with their captured frame.
/// It intentionally talks directly to Dystil's SQLite pool rather than the
/// vendor `DatabaseManager` write queue.
pub struct DystilLinkerRuntime {
    sender: Option<DystilLinkerSender>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl DystilLinkerRuntime {
    pub fn start(pool: SqlitePool) -> Self {
        let (sender, receiver) = mpsc::channel(LINKER_CHANNEL_BUFFER);
        let next_id = Arc::new(AtomicU64::new(1));
        let stop = Arc::new(AtomicBool::new(false));
        let join = tokio::spawn(run_linker(pool, receiver, stop.clone()));
        Self {
            sender: Some(DystilLinkerSender { sender, next_id }),
            stop,
            join: Some(join),
        }
    }

    pub fn sender(&self) -> DystilLinkerSender {
        self.sender
            .as_ref()
            .expect("frame linker sender requested after shutdown")
            .clone()
    }

    pub async fn shutdown(mut self) {
        self.sender.take();
        if let Some(mut join) = self.join.take() {
            if tokio::time::timeout(Duration::from_secs(6), &mut join)
                .await
                .is_err()
            {
                self.stop.store(true, Ordering::Relaxed);
                join.abort();
                let _ = join.await;
            }
        }
    }
}

impl Drop for DystilLinkerRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

async fn run_linker(
    pool: SqlitePool,
    mut receiver: mpsc::Receiver<LinkerMessage>,
    stop: Arc<AtomicBool>,
) {
    let mut pending_events: HashMap<u64, (i64, Instant)> = HashMap::new();
    let mut pending_frames: HashMap<u64, (i64, Instant)> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.tick().await;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        tokio::select! {
            message = receiver.recv() => match message {
                None => break,
                Some(LinkerMessage::EventPersisted { correlation_id, row_id }) => {
                    if let Some((frame_id, _)) = pending_frames.remove(&correlation_id) {
                        apply_update(&pool, row_id, frame_id).await;
                    } else {
                        bounded_insert(&mut pending_events, correlation_id, (row_id, Instant::now()));
                    }
                }
                Some(LinkerMessage::FrameCaptured { frame_id, correlation_ids }) => {
                    for correlation_id in correlation_ids {
                        if let Some((row_id, _)) = pending_events.remove(&correlation_id) {
                            apply_update(&pool, row_id, frame_id).await;
                        } else {
                            bounded_insert(&mut pending_frames, correlation_id, (frame_id, Instant::now()));
                        }
                    }
                }
                Some(LinkerMessage::TriggerDropped { correlation_ids, reason }) => {
                    debug!(?reason, count = correlation_ids.len(), "Dystil frame trigger dropped");
                }
            },
            _ = tick.tick() => {
                let now = Instant::now();
                pending_events.retain(|_, (_, inserted_at)| is_within_ttl(now, *inserted_at));
                pending_frames.retain(|_, (_, inserted_at)| is_within_ttl(now, *inserted_at));
            }
        }
    }
}

fn is_within_ttl(now: Instant, inserted_at: Instant) -> bool {
    now.checked_duration_since(inserted_at)
        .is_none_or(|age| age <= LINKER_TTL)
}

fn bounded_insert<T>(entries: &mut HashMap<u64, (T, Instant)>, id: u64, value: (T, Instant)) {
    if entries.len() >= LINKER_CAPACITY {
        if let Some(oldest) = entries
            .iter()
            .min_by_key(|(_, (_, instant))| *instant)
            .map(|(id, _)| *id)
        {
            entries.remove(&oldest);
        }
    }
    entries.insert(id, value);
}

async fn apply_update(pool: &SqlitePool, row_id: i64, frame_id: i64) {
    #[cfg(feature = "debug-capture")]
    let rss_before = crate::debug_capture::process_rss_bytes();
    #[cfg(feature = "debug-capture")]
    let started = std::time::Instant::now();
    if let Err(error) =
        sqlx::query("UPDATE ui_events SET frame_id = ?1 WHERE id = ?2 AND frame_id IS NULL")
            .bind(frame_id)
            .bind(row_id)
            .execute(pool)
            .await
    {
        warn!(%error, row_id, frame_id, "Dystil frame linker update failed");
    }
    #[cfg(feature = "debug-capture")]
    crate::debug_capture::record_capture_phase(
        "ui_event_activity_linking",
        "link",
        started,
        None,
        None,
        None,
        None,
        None,
        rss_before,
        crate::debug_capture::process_rss_bytes(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_check_does_not_subtract_from_a_fresh_instant() {
        let inserted_at = Instant::now();

        assert!(is_within_ttl(inserted_at, inserted_at));
        assert!(is_within_ttl(inserted_at + LINKER_TTL, inserted_at));
        assert!(!is_within_ttl(
            inserted_at + LINKER_TTL + Duration::from_millis(1),
            inserted_at,
        ));
        assert!(is_within_ttl(
            inserted_at,
            inserted_at + Duration::from_millis(1),
        ));
    }

    async fn frame_id(pool: &SqlitePool, row_id: i64) -> Option<i64> {
        sqlx::query_scalar("SELECT frame_id FROM ui_events WHERE id = ?1")
            .bind(row_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn wait_for_frame_id(pool: &SqlitePool, row_id: i64, expected: i64) {
        for _ in 0..50 {
            if frame_id(pool, row_id).await == Some(expected) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("frame linkage did not arrive");
    }

    #[tokio::test]
    async fn links_event_when_frame_arrives_first_or_last() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE ui_events (id INTEGER PRIMARY KEY, frame_id INTEGER)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO ui_events (id) VALUES (1), (2)")
            .execute(&pool)
            .await
            .unwrap();

        let runtime = DystilLinkerRuntime::start(pool.clone());
        let sender = runtime.sender();
        let recorder = runtime.sender();

        sender.frame_captured(41, vec![7]);
        recorder.event_persisted(7, 1);
        recorder.event_persisted(8, 2);
        sender.frame_captured(42, vec![8]);

        wait_for_frame_id(&pool, 1, 41).await;
        wait_for_frame_id(&pool, 2, 42).await;
        runtime.shutdown().await;
    }
}

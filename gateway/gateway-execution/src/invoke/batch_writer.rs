//! # Batch Writer
//!
//! Decouples hot-path DB writes from stream event processing by batching
//! token updates and log entries into a background task.
//!
//! Instead of writing to SQLite synchronously during stream callbacks,
//! callers send write requests through an mpsc channel. The background
//! task coalesces token updates (keeping only the latest per execution)
//! and batch-inserts log entries.

use api_logs::{ExecutionLog, LogService};
use execution_state::StateService;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use zbot_conversation::MessageStore;
use zbot_runtime_sqlite::DatabaseManager;
use zbot_trace::TraceWriter;

/// An appended row on a session's conversation stream.
#[derive(Debug, Clone)]
pub struct SessionMessage {
    pub session_id: String,
    pub execution_id: String,
    pub role: String,
    pub content: String,
    pub tool_calls: Option<String>,
    pub tool_call_id: Option<String>,
}

/// A write request for the batch writer.
pub enum BatchWrite {
    /// Update token counts for an execution.
    /// Coalesced: only the latest values per execution_id are persisted.
    TokenUpdate {
        execution_id: String,
        tokens_in: u64,
        tokens_out: u64,
    },

    /// Persist an execution log entry.
    LogEntry(ExecutionLog),

    /// Append a message to a session's conversation stream.
    SessionMessage(SessionMessage),

    /// Append a full-fidelity trace event to the session's `.jsonl.gz`.
    TraceEvent {
        session_id: String,
        event: zbot_trace::TraceEvent,
    },

    /// Finalize a session's trace writer (called on session end).
    CloseSessionTrace { session_id: String },

    /// Persist every queued write before acknowledging the caller. This is a
    /// lifecycle barrier used immediately before terminal events so a client
    /// snapshot cannot race the final assistant message.
    Flush {
        acknowledgement: oneshot::Sender<()>,
    },
}

/// Handle for sending writes to the batch writer.
///
/// Cheap to clone. Sending is non-blocking (unbounded channel).
#[derive(Clone)]
pub struct BatchWriterHandle {
    tx: mpsc::UnboundedSender<BatchWrite>,
}

impl BatchWriterHandle {
    /// Send a write request to the batch writer.
    ///
    /// Returns immediately. The write will be persisted asynchronously.
    pub fn send(&self, write: BatchWrite) {
        if self.tx.send(write).is_err() {
            tracing::warn!("BatchWriter channel closed, write dropped");
        }
    }

    /// Convenience: send a token update.
    pub fn token_update(&self, execution_id: &str, tokens_in: u64, tokens_out: u64) {
        self.send(BatchWrite::TokenUpdate {
            execution_id: execution_id.to_string(),
            tokens_in,
            tokens_out,
        });
    }

    /// Convenience: send a log entry.
    pub fn log(&self, entry: ExecutionLog) {
        self.send(BatchWrite::LogEntry(entry));
    }

    /// Convenience: append a message to the session conversation stream.
    pub fn session_message(
        &self,
        session_id: &str,
        execution_id: &str,
        role: &str,
        content: &str,
        tool_calls: Option<&str>,
        tool_call_id: Option<&str>,
    ) {
        self.send(BatchWrite::SessionMessage(SessionMessage {
            session_id: session_id.to_string(),
            execution_id: execution_id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            tool_calls: tool_calls.map(String::from),
            tool_call_id: tool_call_id.map(String::from),
        }));
    }

    /// Convenience: append a full-fidelity trace event to the session's
    /// `.jsonl.gz` (no-op until a `traces_dir` is wired via
    /// `spawn_batch_writer_with_traces`).
    pub fn trace_event(&self, session_id: &str, event: zbot_trace::TraceEvent) {
        self.send(BatchWrite::TraceEvent {
            session_id: session_id.to_string(),
            event,
        });
    }

    /// Convenience: finalize a session's trace writer on session end.
    pub fn close_session_trace(&self, session_id: &str) {
        self.send(BatchWrite::CloseSessionTrace {
            session_id: session_id.to_string(),
        });
    }

    /// Wait until every write queued before this call has been persisted.
    ///
    /// The acknowledgement travels through the same FIFO channel as writes,
    /// so it cannot overtake the terminal assistant message.
    pub async fn flush(&self) {
        let (acknowledgement, received) = oneshot::channel();
        if self.tx.send(BatchWrite::Flush { acknowledgement }).is_err() {
            tracing::warn!("BatchWriter channel closed, terminal flush skipped");
            return;
        }
        if received.await.is_err() {
            tracing::warn!("BatchWriter stopped before terminal flush completed");
        }
    }
}

/// Spawn a batch writer background task.
///
/// Returns a handle for sending writes. The background task runs until
/// the handle (and all clones) are dropped, at which point it flushes
/// remaining writes and exits.
pub fn spawn_batch_writer(
    state_service: Arc<StateService<DatabaseManager>>,
    log_service: Arc<LogService<DatabaseManager>>,
    messages: Arc<dyn MessageStore>,
) -> BatchWriterHandle {
    spawn_batch_writer_inner(state_service, log_service, None, messages)
}

/// Spawn a batch writer that also streams full-fidelity trace events to
/// per-session `.jsonl.gz` files under `traces_dir`.
///
/// Session-message writes route through `MessageStore::append`.
pub fn spawn_batch_writer_with_traces(
    state_service: Arc<StateService<DatabaseManager>>,
    log_service: Arc<LogService<DatabaseManager>>,
    traces_dir: PathBuf,
    messages: Arc<dyn MessageStore>,
) -> BatchWriterHandle {
    if let Err(error) = std::fs::create_dir_all(&traces_dir) {
        tracing::warn!(
            traces_dir = %traces_dir.display(),
            %error,
            "failed to create lazy trace directory"
        );
    }
    spawn_batch_writer_inner(state_service, log_service, Some(traces_dir), messages)
}

fn spawn_batch_writer_inner(
    state_service: Arc<StateService<DatabaseManager>>,
    log_service: Arc<LogService<DatabaseManager>>,
    traces_dir: Option<PathBuf>,
    messages: Arc<dyn MessageStore>,
) -> BatchWriterHandle {
    let (tx, rx) = mpsc::unbounded_channel();

    tokio::spawn(batch_writer_loop(
        rx,
        state_service,
        log_service,
        traces_dir,
        messages,
    ));

    BatchWriterHandle { tx }
}

/// Background loop that processes batched writes.
async fn batch_writer_loop(
    mut rx: mpsc::UnboundedReceiver<BatchWrite>,
    state_service: Arc<StateService<DatabaseManager>>,
    log_service: Arc<LogService<DatabaseManager>>,
    traces_dir: Option<PathBuf>,
    messages: Arc<dyn MessageStore>,
) {
    // Pending token updates — coalesced by execution_id (only latest kept)
    let mut token_updates: HashMap<String, (u64, u64)> = HashMap::new();
    // Pending log entries
    let mut log_entries: Vec<ExecutionLog> = Vec::new();
    // Pending session messages (NOT coalesced — each is unique)
    let mut session_messages: Vec<SessionMessage> = Vec::new();
    // Per-session trace writers (`<session_id>.jsonl.gz`). Each append writes a
    // complete gzip member, so a dropped writer leaves prior events durable.
    let mut trace_writers: HashMap<String, TraceWriter> = HashMap::new();

    let mut interval = tokio::time::interval(Duration::from_millis(100));
    // Don't accumulate ticks while we're busy flushing
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Some(BatchWrite::TokenUpdate { execution_id, tokens_in, tokens_out }) => {
                        // Coalesce: overwrite previous value for this execution
                        token_updates.insert(execution_id, (tokens_in, tokens_out));
                    }
                    Some(BatchWrite::LogEntry(entry)) => {
                        log_entries.push(entry);
                    }
                    Some(BatchWrite::SessionMessage(msg)) => {
                        session_messages.push(msg);
                    }
                    Some(BatchWrite::TraceEvent { session_id, event }) => {
                        if let Some(dir) = traces_dir.as_ref() {
                            match trace_writers.entry(session_id.clone()) {
                                std::collections::hash_map::Entry::Occupied(mut e) => {
                                    if let Err(err) = e.get_mut().append(&event) {
                                        tracing::warn!(
                                            "BatchWriter: trace append failed for {session_id}: {err}"
                                        );
                                    }
                                }
                                std::collections::hash_map::Entry::Vacant(e) => {
                                    match TraceWriter::open_confined(dir, &session_id) {
                                        Ok(mut w) => {
                                            if let Err(err) = w.append(&event) {
                                                tracing::warn!(
                                                    "BatchWriter: trace append failed for {session_id}: {err}"
                                                );
                                            }
                                            e.insert(w);
                                        }
                                        Err(err) => tracing::warn!(
                                            "BatchWriter: trace open failed for {session_id}: {err}"
                                        ),
                                    }
                                }
                            }
                        } else {
                            tracing::trace!("BatchWriter: trace event dropped (no traces_dir)");
                        }
                    }
                    Some(BatchWrite::CloseSessionTrace { session_id }) => {
                        // Dropping the writer finalizes the file; events are
                        // durable (gzip member-per-event).
                        trace_writers.remove(&session_id);
                    }
                    Some(BatchWrite::Flush { acknowledgement }) => {
                        flush_all(&state_service, &log_service, messages.as_ref(), &mut token_updates, &mut log_entries, &mut session_messages);
                        let _ = acknowledgement.send(());
                    }
                    None => {
                        // Channel closed — flush remaining and exit
                        flush_all(&state_service, &log_service, messages.as_ref(), &mut token_updates, &mut log_entries, &mut session_messages);
                        tracing::debug!("BatchWriter shutting down after final flush");
                        return;
                    }
                }

                // Flush if we've accumulated enough items
                let total = token_updates.len() + log_entries.len() + session_messages.len();
                if total >= 10 {
                    flush_all(&state_service, &log_service, messages.as_ref(), &mut token_updates, &mut log_entries, &mut session_messages);
                }
            }
            _ = interval.tick() => {
                // Periodic flush
                if !token_updates.is_empty() || !log_entries.is_empty() || !session_messages.is_empty() {
                    flush_all(&state_service, &log_service, messages.as_ref(), &mut token_updates, &mut log_entries, &mut session_messages);
                }
            }
        }
    }
}

/// Flush all pending writes to the database.
///
/// Session messages are routed through `MessageStore::append`.
fn flush_all(
    state_service: &StateService<DatabaseManager>,
    log_service: &LogService<DatabaseManager>,
    messages: &dyn MessageStore,
    token_updates: &mut HashMap<String, (u64, u64)>,
    log_entries: &mut Vec<ExecutionLog>,
    session_messages: &mut Vec<SessionMessage>,
) {
    // Flush token updates (coalesced — one write per execution)
    for (execution_id, (tokens_in, tokens_out)) in token_updates.drain() {
        if let Err(e) = state_service.update_execution_tokens(&execution_id, tokens_in, tokens_out)
        {
            tracing::warn!(
                "BatchWriter: failed to update tokens for {}: {}",
                execution_id,
                e
            );
        }
    }

    // Flush log entries
    for entry in log_entries.drain(..) {
        if let Err(e) = log_service.log(entry) {
            tracing::warn!("BatchWriter: failed to write log: {}", e);
        }
    }

    for msg in session_messages.drain(..) {
        let message = zbot_conversation::Message {
            id: format!("msg-{}", uuid::Uuid::new_v4()),
            execution_id: Some(msg.execution_id.clone()),
            session_id: msg.session_id.clone(),
            role: msg.role.clone(),
            content: msg.content.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            token_count: msg.content.len() as i64 / 4,
            tool_calls: msg.tool_calls.clone(),
            tool_call_id: msg.tool_call_id.clone(),
            seq: 0, // assigned atomically inside append; ignored server-side
        };
        if let Err(e) = messages.append(&message) {
            tracing::warn!(
                "BatchWriter: failed to append message via MessageStore: {}",
                e
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api_logs::{LogCategory, LogLevel};
    use gateway_services::VaultPaths;
    use tempfile::TempDir;

    /// Full wiring: temp vault, real DB, services, a seeded session/execution
    /// so FKs (artifacts / token updates / session_messages) are satisfied.
    struct Harness {
        _tmp: TempDir,
        state: Arc<StateService<DatabaseManager>>,
        logs: Arc<LogService<DatabaseManager>>,
        messages: Arc<dyn MessageStore>,
        session_id: String,
        execution_id: String,
    }

    fn setup() -> Harness {
        let tmp = TempDir::new().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().expect("ensure vault dirs");
        let db = Arc::new(DatabaseManager::new(paths.clone()).expect("db init"));
        let state = Arc::new(StateService::new(db.clone()));
        let logs = Arc::new(LogService::new(db.clone()));
        let pool = zbot_conversation::open_conversation_pool(&paths.conversations_db())
            .expect("conversation pool");
        let messages = Arc::new(zbot_conversation::SqliteMessageStore::new(pool));
        let (session, execution) = state.create_session("agent-test").expect("seed session");
        Harness {
            _tmp: tmp,
            state,
            logs,
            messages,
            session_id: session.id,
            execution_id: execution.id,
        }
    }

    // ------------------------------------------------------------------
    // Handle: channel behaviour + convenience methods serialize correctly
    // ------------------------------------------------------------------

    #[test]
    fn send_on_closed_channel_does_not_panic() {
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx); // receiver gone — any send is a dead-letter
        let handle = BatchWriterHandle { tx };
        // Covers both the convenience method and the silent-drop branch in
        // BatchWriterHandle::send.
        handle.token_update("e1", 1, 2);
        handle.log(ExecutionLog::new(
            "s1",
            "c1",
            "a1",
            LogLevel::Info,
            LogCategory::Session,
            "msg",
        ));
        handle.session_message("s1", "e1", "user", "hi", None, None);
    }

    #[tokio::test]
    async fn convenience_methods_enqueue_correct_variants() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = BatchWriterHandle { tx };

        handle.token_update("exec-9", 11, 22);
        match rx.recv().await.expect("token_update queued") {
            BatchWrite::TokenUpdate {
                execution_id,
                tokens_in,
                tokens_out,
            } => {
                assert_eq!(execution_id, "exec-9");
                assert_eq!(tokens_in, 11);
                assert_eq!(tokens_out, 22);
            }
            other => panic!(
                "expected TokenUpdate, got {:?}",
                std::mem::discriminant(&other)
            ),
        }

        handle.session_message("s1", "e1", "user", "hello", Some("tc"), Some("tcid"));
        match rx.recv().await.expect("session_message queued") {
            BatchWrite::SessionMessage(msg) => {
                assert_eq!(msg.session_id, "s1");
                assert_eq!(msg.role, "user");
                assert_eq!(msg.content, "hello");
                assert_eq!(msg.tool_calls.as_deref(), Some("tc"));
                assert_eq!(msg.tool_call_id.as_deref(), Some("tcid"));
            }
            other => panic!(
                "expected SessionMessage, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[tokio::test]
    async fn flush_makes_a_queued_session_message_immediately_replayable() {
        let h = setup();
        let writer = spawn_batch_writer(h.state.clone(), h.logs.clone(), h.messages.clone());

        writer.session_message(
            &h.session_id,
            &h.execution_id,
            "assistant",
            "terminal response",
            None,
            None,
        );
        writer.flush().await;

        let messages = h.messages.replay(&h.session_id, None, 100).expect("replay");
        assert!(messages
            .iter()
            .any(|message| message.content == "terminal response"));
    }

    // ------------------------------------------------------------------
    // Loop: runs until channel closes, flushes on shutdown
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn loop_exits_and_flushes_when_channel_closes() {
        let h = setup();
        let (tx, rx) = mpsc::unbounded_channel();

        let task = tokio::spawn(batch_writer_loop(
            rx,
            h.state.clone(),
            h.logs.clone(),
            None,
            h.messages.clone(),
        ));

        // Enqueue a log and a session message. Neither is on the 10-item fast
        // path — the shutdown-flush branch is what persists them.
        tx.send(BatchWrite::LogEntry(ExecutionLog::new(
            &h.session_id,
            "conv-1",
            "agent-test",
            LogLevel::Info,
            LogCategory::Session,
            "hello",
        )))
        .expect("send log");
        tx.send(BatchWrite::SessionMessage(SessionMessage {
            session_id: h.session_id.clone(),
            execution_id: h.execution_id.clone(),
            role: "user".into(),
            content: "from-batch".into(),
            tool_calls: None,
            tool_call_id: None,
        }))
        .expect("send msg");

        drop(tx);
        task.await.expect("task joins cleanly");

        // Final-flush branch must have written the session message.
        let msgs = h.messages.replay(&h.session_id, None, 100).expect("replay");
        assert!(
            msgs.iter().any(|m| m.content == "from-batch"),
            "expected flushed session message in {msgs:?}"
        );
    }

    #[tokio::test]
    async fn token_updates_coalesce_to_latest_value_per_execution() {
        let h = setup();
        let (tx, rx) = mpsc::unbounded_channel();

        let task = tokio::spawn(batch_writer_loop(
            rx,
            h.state.clone(),
            h.logs.clone(),
            None,
            h.messages.clone(),
        ));

        for (tin, tout) in [(1, 2), (3, 4), (5, 6), (7, 8)] {
            tx.send(BatchWrite::TokenUpdate {
                execution_id: h.execution_id.clone(),
                tokens_in: tin,
                tokens_out: tout,
            })
            .expect("send");
        }
        drop(tx);
        task.await.expect("task joins");

        let execution = h
            .state
            .get_execution(&h.execution_id)
            .expect("get_execution")
            .expect("execution exists");
        assert_eq!(
            execution.tokens_in, 7,
            "coalesce must keep the LAST tokens_in"
        );
        assert_eq!(
            execution.tokens_out, 8,
            "coalesce must keep the LAST tokens_out"
        );
    }

    #[tokio::test]
    async fn count_threshold_flushes_mid_loop() {
        let h = setup();
        let (tx, rx) = mpsc::unbounded_channel();

        let task = tokio::spawn(batch_writer_loop(
            rx,
            h.state.clone(),
            h.logs.clone(),
            None,
            h.messages.clone(),
        ));

        // Ten session messages pushes the pending-count gate at ≥10. The
        // in-loop flush branch must fire before channel close.
        for i in 0..10 {
            tx.send(BatchWrite::SessionMessage(SessionMessage {
                session_id: h.session_id.clone(),
                execution_id: h.execution_id.clone(),
                role: "user".into(),
                content: format!("msg-{i}"),
                tool_calls: None,
                tool_call_id: None,
            }))
            .expect("send");
        }
        drop(tx);
        task.await.expect("task joins");

        let msgs = h.messages.replay(&h.session_id, None, 100).expect("replay");
        // 10 sent, 10 must land. Order preserved by the Vec.
        let batch_msgs: Vec<_> = msgs
            .iter()
            .filter(|m| m.content.starts_with("msg-"))
            .collect();
        assert_eq!(batch_msgs.len(), 10);
        for (i, m) in batch_msgs.iter().enumerate() {
            assert_eq!(m.content, format!("msg-{i}"));
        }
    }

    #[tokio::test]
    async fn spawn_batch_writer_returns_working_handle() {
        // Integration smoke test of the public spawn helpers — just covers the
        // `spawn_batch_writer` entry point so it is not 0%.
        let h = setup();
        let handle = spawn_batch_writer(h.state.clone(), h.logs.clone(), h.messages.clone());
        handle.token_update(&h.execution_id, 100, 200);
        drop(handle);

        // Give the spawned task a moment to observe channel close + flush.
        // 300ms comfortably exceeds the 100ms periodic tick + flush_all work.
        tokio::time::sleep(Duration::from_millis(300)).await;

        let execution = h
            .state
            .get_execution(&h.execution_id)
            .expect("get_execution")
            .expect("execution exists");
        assert_eq!(execution.tokens_in, 100);
        assert_eq!(execution.tokens_out, 200);
    }

    fn trace_ev(span: &str) -> zbot_trace::TraceEvent {
        zbot_trace::TraceEvent {
            trace_id: "tr".into(),
            span_id: span.into(),
            session_id: "s-trace".into(),
            execution_id: "e1".into(),
            agent_id: "root".into(),
            parent_session_id: None,
            timestamp: "2026-07-07T00:00:00Z".into(),
            level: "info".into(),
            category: "tool_call".into(),
            message: format!("ev {span}"),
            duration_ms: None,
            tool_name: Some("read_file".into()),
            payload: None,
            usage: None,
            model: None,
        }
    }

    #[tokio::test]
    async fn trace_events_stream_to_jsonl_gz() {
        let h = setup();
        let traces_dir = h._tmp.path().join("data").join("traces");
        // `batch_writer_loop` is intentionally lower-level than its public
        // spawn helper, so this fixture creates the trace sink explicitly.
        std::fs::create_dir_all(&traces_dir).expect("create lazy trace directory");

        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(batch_writer_loop(
            rx,
            h.state.clone(),
            h.logs.clone(),
            Some(traces_dir.clone()),
            h.messages.clone(),
        ));

        tx.send(BatchWrite::TraceEvent {
            session_id: "s-trace".into(),
            event: trace_ev("a"),
        })
        .expect("send a");
        tx.send(BatchWrite::TraceEvent {
            session_id: "s-trace".into(),
            event: trace_ev("b"),
        })
        .expect("send b");
        tx.send(BatchWrite::CloseSessionTrace {
            session_id: "s-trace".into(),
        })
        .expect("close");
        drop(tx);
        task.await.expect("task joins");

        // The session's .jsonl.gz holds both events (gzip member-per-event).
        let path = traces_dir.join("s-trace.jsonl.gz");
        let bytes = std::fs::read(&path).expect("trace file exists");
        use std::io::Read;
        let mut dec = flate2::read::MultiGzDecoder::new(&bytes[..]);
        let mut out = String::new();
        dec.read_to_string(&mut out).expect("decode");
        let lines: Vec<&str> = out.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 2, "two trace events decoded");
        assert!(lines[0].contains(r#""span_id":"a""#));
        assert!(lines[1].contains(r#""span_id":"b""#));
    }
}

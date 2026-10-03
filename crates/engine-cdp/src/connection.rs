//! Minimal Chrome DevTools Protocol client over one WebSocket: request/response
//! correlation by `id`, events fanned out to per-session queues.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Notify};
use tokio_tungstenite::tungstenite::Message;

use crate::CdpError;

#[derive(Debug, Clone)]
pub struct CdpEvent {
    /// Monotonic arrival number across all sessions; lets callers wait only
    /// for events newer than a point in time (see [`Connection::cursor`]).
    pub seq: u64,
    pub session_id: Option<String>,
    pub method: String,
    pub params: Value,
}

/// Position in the event stream, as returned by [`Connection::cursor`].
pub type Cursor = u64;

#[derive(Default)]
struct EventQueues {
    /// Events per session (`None` = browser-level).
    by_session: HashMap<Option<String>, VecDeque<CdpEvent>>,
}

pub struct Connection {
    tx: mpsc::UnboundedSender<String>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CdpError>>>>>,
    events: Arc<Mutex<EventQueues>>,
    event_notify: Arc<Notify>,
    next_id: AtomicU64,
    next_seq: Arc<AtomicU64>,
    closed: Arc<std::sync::atomic::AtomicBool>,
}

const MAX_QUEUED_EVENTS: usize = 2000;

impl Connection {
    pub async fn connect(ws_url: &str) -> Result<Arc<Connection>, CdpError> {
        let (stream, _) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|e| CdpError::Transport(format!("websocket connect failed: {e}")))?;
        let (mut sink, mut source) = stream.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CdpError>>>>> = Default::default();
        let events: Arc<Mutex<EventQueues>> = Default::default();
        let event_notify = Arc::new(Notify::new());
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let next_seq = Arc::new(AtomicU64::new(1));

        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if sink.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }
        });

        let pending_r = pending.clone();
        let events_r = events.clone();
        let notify_r = event_notify.clone();
        let closed_r = closed.clone();
        let seq_r = next_seq.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = source.next().await {
                let Message::Text(text) = msg else { continue };
                let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                if let Some(id) = v.get("id").and_then(Value::as_u64) {
                    if let Some(tx) = pending_r.lock().unwrap().remove(&id) {
                        let res = match v.get("error") {
                            Some(err) => Err(CdpError::Protocol {
                                code: err.get("code").and_then(Value::as_i64).unwrap_or(0),
                                message: err.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
                                data: err.get("data").and_then(Value::as_str).map(str::to_string),
                            }),
                            None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        let _ = tx.send(res);
                    }
                } else if let Some(method) = v.get("method").and_then(Value::as_str) {
                    let ev = CdpEvent {
                        seq: seq_r.fetch_add(1, Ordering::SeqCst),
                        session_id: v.get("sessionId").and_then(Value::as_str).map(str::to_string),
                        method: method.to_string(),
                        params: v.get("params").cloned().unwrap_or(Value::Null),
                    };
                    let mut q = events_r.lock().unwrap();
                    let queue = q.by_session.entry(ev.session_id.clone()).or_default();
                    if queue.len() >= MAX_QUEUED_EVENTS {
                        queue.pop_front();
                    }
                    queue.push_back(ev);
                    drop(q);
                    notify_r.notify_waiters();
                }
            }
            closed_r.store(true, Ordering::SeqCst);
            // Fail every outstanding call so nobody waits forever.
            for (_, tx) in pending_r.lock().unwrap().drain() {
                let _ = tx.send(Err(CdpError::Transport("connection closed".into())));
            }
            notify_r.notify_waiters();
        });

        Ok(Arc::new(Connection { tx, pending, events, event_notify, next_id: AtomicU64::new(1), next_seq, closed }))
    }

    /// Current position in the event stream. Events received after this call
    /// have `seq >= cursor`.
    pub fn cursor(&self) -> Cursor {
        self.next_seq.load(Ordering::SeqCst)
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub async fn call(&self, session: Option<&str>, method: &str, params: Value) -> Result<Value, CdpError> {
        if self.is_closed() {
            return Err(CdpError::Transport("connection closed".into()));
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.tx.send(msg.to_string()).map_err(|_| CdpError::Transport("send failed".into()))?;
        match tokio::time::timeout(Duration::from_secs(60), rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(CdpError::Transport("response channel dropped".into())),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(CdpError::Timeout(format!("{method} timed out")))
            }
        }
    }

    /// Drain queued events for a session.
    pub fn drain_events(&self, session: Option<&str>) -> Vec<CdpEvent> {
        let mut q = self.events.lock().unwrap();
        q.by_session.get_mut(&session.map(str::to_string)).map(|d| d.drain(..).collect()).unwrap_or_default()
    }

    /// Wait until an event for `session` that arrived at or after `since`
    /// satisfies `pred`, or the timeout elapses. Events are left in the queue
    /// for `drain_events`, so waiting never hides anything from `poll_events`.
    pub async fn wait_for(
        &self,
        session: Option<&str>,
        since: Cursor,
        timeout: Duration,
        mut pred: impl FnMut(&CdpEvent) -> bool,
    ) -> Option<CdpEvent> {
        let deadline = tokio::time::Instant::now() + timeout;
        let key = session.map(str::to_string);
        let mut checked_up_to = since;
        loop {
            // Register interest before scanning so an event that lands between
            // the scan and the await still wakes us (`notify_waiters` only
            // wakes already-registered waiters).
            let notified = self.event_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let q = self.events.lock().unwrap();
                if let Some(d) = q.by_session.get(&key) {
                    let floor = checked_up_to;
                    for ev in d.iter().filter(|e| e.seq >= floor) {
                        if pred(ev) {
                            return Some(ev.clone());
                        }
                        checked_up_to = ev.seq + 1;
                    }
                }
            }
            if self.is_closed() {
                return None;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return None;
            }
        }
    }
}

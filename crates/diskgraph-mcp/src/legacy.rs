//! Legacy HTTP+SSE transport (P4 tasks 5.6 / 5.7, specs MCP-01 / MCP-04).
//!
//! An adapter for the pre-Streamable MCP wire protocol: the client opens
//! `GET /sse`, receives an `endpoint` event naming its message URL, then POSTs
//! JSON-RPC to that URL and receives responses as `message` events on the
//! original stream. The adapter is deliberately separate from the modern
//! transport: it is disabled by default, it shares no routing with `/mcp`, and
//! it answers the same service under the same authorization — a tool that is
//! refused over the modern transport is refused here too.

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// Where a legacy client connects.
pub const LEGACY_SSE_PATH: &str = "/sse";
/// The message-posting path the endpoint event advertises. Sessions are
/// addressed by query parameter, exactly as the 2024-11-05 protocol spells it.
pub const LEGACY_MESSAGES_PATH: &str = "/messages";

/// Formats the endpoint event a legacy client waits for on its SSE stream.
pub fn endpoint_event(session_id: &str) -> String {
    format!("event: endpoint\ndata: {LEGACY_MESSAGES_PATH}/?session_id={session_id}\n\n")
}

/// Formats one JSON-RPC response as a `message` event.
pub fn message_event(payload: &str) -> String {
    // The payload is compact JSON produced by this server; it carries no CR or
    // LF, so one data line is always a complete event.
    format!("event: message\ndata: {payload}\n\n")
}

/// A liveness tick written so idle streams survive intermediaries that reap
/// silent connections. Comments are ignored by SSE clients.
pub fn keepalive_event() -> String {
    ": keep-alive\n\n".to_owned()
}

/// Live legacy sessions and the channel each SSE connection drains.
type SessionChannel = (Option<String>, mpsc::SyncSender<String>);

#[derive(Clone, Default)]
pub struct SessionRegistry {
    inner: Arc<Mutex<HashMap<String, SessionChannel>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens one session; the receiver belongs to the SSE connection thread.
    pub fn open(&self) -> (String, mpsc::Receiver<String>) {
        self.open_bound(None)
    }

    /// 绑定认证主体并限制待发送队列，防止跨主体注入和无界积压。
    pub fn open_bound(&self, principal: Option<String>) -> (String, mpsc::Receiver<String>) {
        let session_id = format!("legacy-{}", uuid::Uuid::new_v4());
        let (sender, receiver) = mpsc::sync_channel(64);
        if let Ok(mut sessions) = self.inner.lock() {
            sessions.insert(session_id.clone(), (principal, sender));
        }
        (session_id, receiver)
    }

    pub fn lookup(&self, session_id: &str) -> Option<mpsc::SyncSender<String>> {
        self.inner
            .lock()
            .ok()?
            .get(session_id)
            .map(|(_, sender)| sender.clone())
    }

    /// 会话仅可由握手时绑定的主体投递。
    pub fn lookup_bound(
        &self,
        session_id: &str,
        principal: Option<&str>,
    ) -> Option<mpsc::SyncSender<String>> {
        let state = self.inner.lock().ok()?;
        let (owner, sender) = state.get(session_id)?;
        (owner.as_deref() == principal).then(|| sender.clone())
    }

    pub fn close(&self, session_id: &str) {
        if let Ok(mut sessions) = self.inner.lock() {
            sessions.remove(session_id);
        }
    }

    pub fn active_count(&self) -> usize {
        self.inner
            .lock()
            .map(|sessions| sessions.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_endpoint_event_names_the_session_message_url() {
        let event = endpoint_event("abc-123");
        assert!(event.starts_with("event: endpoint\n"));
        assert!(event.contains("data: /messages/?session_id=abc-123"));
        assert!(event.ends_with("\n\n"));
    }

    #[test]
    fn message_events_carry_one_complete_json_line() {
        let payload = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
        let event = message_event(payload);
        assert_eq!(event, format!("event: message\ndata: {payload}\n\n"));
        // The payload itself is a single line: no CR or LF inside the data.
        assert!(!payload.contains(['\r', '\n']));
    }

    #[test]
    fn sessions_round_trip_and_close() {
        let registry = SessionRegistry::new();
        let (id, _receiver) = registry.open();
        assert_eq!(registry.active_count(), 1);
        assert!(registry.lookup(&id).is_some());
        registry.close(&id);
        assert_eq!(registry.active_count(), 0);
        assert!(registry.lookup(&id).is_none());
    }

    #[test]
    fn a_dropped_sse_end_closes_the_session_for_posters() {
        let registry = SessionRegistry::new();
        let (id, receiver) = registry.open();
        drop(receiver);
        // With the SSE end gone, a send fails immediately instead of queueing
        // forever; the registry entry is cleaned up by the SSE thread.
        let sender = registry.lookup(&id).unwrap();
        assert!(sender.send("x".into()).is_err());
        registry.close(&id);
        assert!(registry.lookup(&id).is_none());
    }
}

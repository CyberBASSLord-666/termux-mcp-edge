//! Bounded in-memory lifecycle state for Streamable HTTP MCP sessions.
//!
//! Session records retain a random identifier, lifecycle phase, last-activity
//! timestamp, and—only when used—fixed-size server-generated SSE replay state.
//! Raw client initialization metadata and presented cursors are never retained;
//! replay stores only bounded serialized server responses.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use uuid::Uuid;

pub(crate) const MAX_MCP_SESSIONS: usize = 64;
pub(crate) const MCP_SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub(crate) const MAX_MCP_SSE_STREAMS_PER_SESSION: usize = 8;
pub(crate) const MAX_MCP_SSE_EVENTS_PER_STREAM: usize = 2;
pub(crate) const MAX_MCP_SSE_EVENT_DATA_BYTES: usize = 128 * 1024;
pub(crate) const MAX_MCP_SSE_REPLAY_BYTES_PER_SESSION: usize = 256 * 1024;
pub(crate) const SSE_RETRY_MILLISECONDS: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionPhase {
    AwaitingInitialized,
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionStoreError {
    CapacityExhausted,
    NotFound,
    Poisoned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SseReplayError {
    SessionNotFound,
    CursorNotFound,
    EventDataTooLarge,
    Poisoned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseReplayEvent {
    id: String,
    data: Vec<u8>,
    retry: bool,
}

impl SseReplayEvent {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn append_wire_bytes(&self, output: &mut Vec<u8>) {
        output.extend_from_slice(b"id: ");
        output.extend_from_slice(self.id.as_bytes());
        if self.retry {
            output.extend_from_slice(b"\nretry: 1000");
        }
        output.extend_from_slice(b"\ndata: ");
        output.extend_from_slice(&self.data);
        output.extend_from_slice(b"\n\n");
    }

    fn wire_len(&self) -> usize {
        b"id: ".len()
            + self.id.len()
            + if self.retry {
                b"\nretry: 1000".len()
            } else {
                0
            }
            + b"\ndata: ".len()
            + self.data.len()
            + b"\n\n".len()
    }
}

#[derive(Clone)]
pub(crate) struct McpSessionStore {
    inner: Arc<Mutex<SessionRegistry>>,
    max_sessions: usize,
    idle_timeout: Duration,
}

#[derive(Default)]
struct SessionRegistry {
    sessions: HashMap<String, SessionRecord>,
}

struct SessionRecord {
    phase: SessionPhase,
    last_activity: Instant,
    sse_streams: VecDeque<SseReplayStream>,
    sse_replay_bytes: usize,
}

struct SseReplayStream {
    events: Vec<SseReplayEvent>,
    retained_bytes: usize,
}

impl McpSessionStore {
    pub(crate) fn new() -> Self {
        Self::with_limits(MAX_MCP_SESSIONS, MCP_SESSION_IDLE_TIMEOUT)
    }

    fn with_limits(max_sessions: usize, idle_timeout: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SessionRegistry::default())),
            max_sessions,
            idle_timeout,
        }
    }

    pub(crate) fn create(&self) -> Result<String, SessionStoreError> {
        self.create_at(Instant::now())
    }

    pub(crate) fn phase(&self, session_id: &str) -> Result<SessionPhase, SessionStoreError> {
        self.phase_at(session_id, Instant::now())
    }

    pub(crate) fn activate(&self, session_id: &str) -> Result<(), SessionStoreError> {
        self.activate_at(session_id, Instant::now())
    }

    pub(crate) fn terminate(&self, session_id: &str) -> Result<(), SessionStoreError> {
        self.terminate_at(session_id, Instant::now())
    }

    pub(crate) fn record_sse_response(
        &self,
        session_id: &str,
        data: Vec<u8>,
    ) -> Result<Vec<SseReplayEvent>, SseReplayError> {
        self.record_sse_response_at(session_id, data, Instant::now())
    }

    pub(crate) fn replay_sse_after(
        &self,
        session_id: &str,
        last_event_id: &str,
    ) -> Result<Vec<SseReplayEvent>, SseReplayError> {
        self.replay_sse_after_at(session_id, last_event_id, Instant::now())
    }

    fn create_at(&self, now: Instant) -> Result<String, SessionStoreError> {
        let mut registry = self.inner.lock().map_err(|_| SessionStoreError::Poisoned)?;
        registry.prune_expired(now, self.idle_timeout);

        if registry.sessions.len() >= self.max_sessions {
            return Err(SessionStoreError::CapacityExhausted);
        }

        let session_id = loop {
            let candidate = Uuid::new_v4().to_string();
            if !registry.sessions.contains_key(&candidate) {
                break candidate;
            }
        };

        registry.sessions.insert(
            session_id.clone(),
            SessionRecord {
                phase: SessionPhase::AwaitingInitialized,
                last_activity: now,
                sse_streams: VecDeque::new(),
                sse_replay_bytes: 0,
            },
        );
        Ok(session_id)
    }

    fn phase_at(&self, session_id: &str, now: Instant) -> Result<SessionPhase, SessionStoreError> {
        let mut registry = self.inner.lock().map_err(|_| SessionStoreError::Poisoned)?;
        registry.prune_expired(now, self.idle_timeout);
        let session = registry
            .sessions
            .get_mut(session_id)
            .ok_or(SessionStoreError::NotFound)?;
        session.last_activity = now;
        Ok(session.phase)
    }

    fn activate_at(&self, session_id: &str, now: Instant) -> Result<(), SessionStoreError> {
        let mut registry = self.inner.lock().map_err(|_| SessionStoreError::Poisoned)?;
        registry.prune_expired(now, self.idle_timeout);
        let session = registry
            .sessions
            .get_mut(session_id)
            .ok_or(SessionStoreError::NotFound)?;
        session.phase = SessionPhase::Active;
        session.last_activity = now;
        Ok(())
    }

    fn terminate_at(&self, session_id: &str, now: Instant) -> Result<(), SessionStoreError> {
        let mut registry = self.inner.lock().map_err(|_| SessionStoreError::Poisoned)?;
        registry.prune_expired(now, self.idle_timeout);
        registry
            .sessions
            .remove(session_id)
            .map(|_| ())
            .ok_or(SessionStoreError::NotFound)
    }

    fn record_sse_response_at(
        &self,
        session_id: &str,
        data: Vec<u8>,
        now: Instant,
    ) -> Result<Vec<SseReplayEvent>, SseReplayError> {
        if data.len() > MAX_MCP_SSE_EVENT_DATA_BYTES {
            return Err(SseReplayError::EventDataTooLarge);
        }

        let mut registry = self.inner.lock().map_err(|_| SseReplayError::Poisoned)?;
        registry.prune_expired(now, self.idle_timeout);
        let session = registry
            .sessions
            .get_mut(session_id)
            .ok_or(SseReplayError::SessionNotFound)?;

        let stream_id = loop {
            let candidate = Uuid::new_v4().to_string();
            if session.sse_streams.iter().all(|stream| {
                stream
                    .events
                    .first()
                    .and_then(|event| event.id.split_once(':'))
                    .is_none_or(|(existing, _)| existing != candidate.as_str())
            }) {
                break candidate;
            }
        };
        let events = vec![
            SseReplayEvent {
                id: format!("{stream_id}:0"),
                data: Vec::new(),
                retry: true,
            },
            SseReplayEvent {
                id: format!("{stream_id}:1"),
                data,
                retry: false,
            },
        ];
        debug_assert_eq!(events.len(), MAX_MCP_SSE_EVENTS_PER_STREAM);
        let retained_bytes = events.iter().map(SseReplayEvent::wire_len).sum::<usize>();
        if retained_bytes > MAX_MCP_SSE_REPLAY_BYTES_PER_SESSION {
            return Err(SseReplayError::EventDataTooLarge);
        }

        while session.sse_streams.len() >= MAX_MCP_SSE_STREAMS_PER_SESSION
            || session.sse_replay_bytes + retained_bytes > MAX_MCP_SSE_REPLAY_BYTES_PER_SESSION
        {
            let evicted = session
                .sse_streams
                .pop_front()
                .expect("a stream limit can only be exceeded with retained streams");
            session.sse_replay_bytes -= evicted.retained_bytes;
        }

        session.sse_replay_bytes += retained_bytes;
        session.sse_streams.push_back(SseReplayStream {
            events: events.clone(),
            retained_bytes,
        });
        session.last_activity = now;
        Ok(events)
    }

    fn replay_sse_after_at(
        &self,
        session_id: &str,
        last_event_id: &str,
        now: Instant,
    ) -> Result<Vec<SseReplayEvent>, SseReplayError> {
        let mut registry = self.inner.lock().map_err(|_| SseReplayError::Poisoned)?;
        registry.prune_expired(now, self.idle_timeout);
        let session = registry
            .sessions
            .get_mut(session_id)
            .ok_or(SseReplayError::SessionNotFound)?;

        let replay = session.sse_streams.iter().find_map(|stream| {
            stream
                .events
                .iter()
                .position(|event| event.id == last_event_id)
                .map(|index| stream.events.iter().skip(index + 1).cloned().collect())
        });
        match replay {
            Some(events) => {
                session.last_activity = now;
                Ok(events)
            }
            None => Err(SseReplayError::CursorNotFound),
        }
    }
}

impl SessionRegistry {
    fn prune_expired(&mut self, now: Instant, idle_timeout: Duration) {
        self.sessions.retain(|_, session| {
            now.checked_duration_since(session.last_activity)
                .is_none_or(|idle| idle < idle_timeout)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_visible_ascii_uuid_sessions_in_pending_phase() {
        let store = McpSessionStore::new();
        let session_id = store.create().unwrap();

        assert_eq!(
            Uuid::parse_str(&session_id).unwrap().to_string(),
            session_id
        );
        assert!(session_id.bytes().all(|byte| (0x21..=0x7e).contains(&byte)));
        assert_eq!(
            store.phase(&session_id).unwrap(),
            SessionPhase::AwaitingInitialized
        );
    }

    #[test]
    fn activation_is_idempotent_and_scoped_to_one_session() {
        let store = McpSessionStore::new();
        let first = store.create().unwrap();
        let second = store.create().unwrap();

        store.activate(&first).unwrap();
        store.activate(&first).unwrap();

        assert_eq!(store.phase(&first).unwrap(), SessionPhase::Active);
        assert_eq!(
            store.phase(&second).unwrap(),
            SessionPhase::AwaitingInitialized
        );
    }

    #[test]
    fn capacity_is_bounded_until_a_session_is_terminated() {
        let store = McpSessionStore::with_limits(2, Duration::from_secs(60));
        let first = store.create().unwrap();
        let _second = store.create().unwrap();

        assert_eq!(
            store.create().unwrap_err(),
            SessionStoreError::CapacityExhausted
        );

        store.terminate(&first).unwrap();
        assert!(store.create().is_ok());
    }

    #[test]
    fn idle_sessions_expire_and_release_capacity() {
        let start = Instant::now();
        let store = McpSessionStore::with_limits(1, Duration::from_secs(10));
        let expired = store.create_at(start).unwrap();

        assert_eq!(
            store
                .phase_at(&expired, start + Duration::from_secs(9))
                .unwrap(),
            SessionPhase::AwaitingInitialized
        );
        assert_eq!(
            store
                .create_at(start + Duration::from_secs(10))
                .unwrap_err(),
            SessionStoreError::CapacityExhausted
        );

        let replacement = store.create_at(start + Duration::from_secs(20)).unwrap();
        assert_ne!(replacement, expired);
        assert_eq!(
            store
                .phase_at(&expired, start + Duration::from_secs(20))
                .unwrap_err(),
            SessionStoreError::NotFound
        );
    }

    #[test]
    fn terminated_and_unknown_sessions_are_not_found() {
        let store = McpSessionStore::new();
        let session_id = store.create().unwrap();

        store.terminate(&session_id).unwrap();

        assert_eq!(
            store.phase(&session_id).unwrap_err(),
            SessionStoreError::NotFound
        );
        assert_eq!(
            store.terminate("not-a-session").unwrap_err(),
            SessionStoreError::NotFound
        );
    }

    #[test]
    fn sse_response_is_primed_bounded_and_replayed_only_after_its_cursor() {
        let store = McpSessionStore::new();
        let session_id = store.create().unwrap();
        let events = store
            .record_sse_response(&session_id, br#"{"jsonrpc":"2.0","id":1}"#.to_vec())
            .unwrap();

        assert_eq!(events.len(), MAX_MCP_SSE_EVENTS_PER_STREAM);
        assert!(events[0].data.is_empty());
        assert!(events[0].retry);
        assert!(!events[1].retry);
        assert_eq!(events[1].data, br#"{"jsonrpc":"2.0","id":1}"#);
        assert_eq!(
            events[0].id.split_once(':').unwrap().0,
            events[1].id.split_once(':').unwrap().0
        );

        assert_eq!(
            store.replay_sse_after(&session_id, events[0].id()).unwrap(),
            vec![events[1].clone()]
        );
        assert!(store
            .replay_sse_after(&session_id, events[1].id())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn sse_cursor_never_crosses_session_or_stream_boundaries() {
        let store = McpSessionStore::new();
        let first = store.create().unwrap();
        let second = store.create().unwrap();
        let first_events = store
            .record_sse_response(&first, b"first".to_vec())
            .unwrap();
        let second_events = store
            .record_sse_response(&second, b"second".to_vec())
            .unwrap();

        assert_eq!(
            store
                .replay_sse_after(&second, first_events[0].id())
                .unwrap_err(),
            SseReplayError::CursorNotFound
        );
        assert_eq!(
            store
                .replay_sse_after(&first, second_events[0].id())
                .unwrap_err(),
            SseReplayError::CursorNotFound
        );
    }

    #[test]
    fn sse_stream_count_and_retained_bytes_evict_oldest_streams() {
        let store = McpSessionStore::new();
        let session_id = store.create().unwrap();
        let first = store
            .record_sse_response(&session_id, b"first".to_vec())
            .unwrap();
        let mut newest = first.clone();
        for index in 0..MAX_MCP_SSE_STREAMS_PER_SESSION {
            newest = store
                .record_sse_response(&session_id, index.to_string().into_bytes())
                .unwrap();
        }
        assert_eq!(
            store
                .replay_sse_after(&session_id, first[0].id())
                .unwrap_err(),
            SseReplayError::CursorNotFound
        );
        assert_eq!(
            store.replay_sse_after(&session_id, newest[0].id()).unwrap(),
            vec![newest[1].clone()]
        );

        let large_first = store
            .record_sse_response(&session_id, vec![b'a'; MAX_MCP_SSE_EVENT_DATA_BYTES])
            .unwrap();
        let large_second = store
            .record_sse_response(&session_id, vec![b'b'; MAX_MCP_SSE_EVENT_DATA_BYTES])
            .unwrap();
        assert_eq!(
            store
                .replay_sse_after(&session_id, large_first[0].id())
                .unwrap_err(),
            SseReplayError::CursorNotFound
        );
        assert_eq!(
            store
                .replay_sse_after(&session_id, large_second[0].id())
                .unwrap(),
            vec![large_second[1].clone()]
        );
    }

    #[test]
    fn oversized_sse_data_is_rejected_without_eviction() {
        let store = McpSessionStore::new();
        let session_id = store.create().unwrap();
        let retained = store
            .record_sse_response(&session_id, b"retained".to_vec())
            .unwrap();

        assert_eq!(
            store
                .record_sse_response(&session_id, vec![0; MAX_MCP_SSE_EVENT_DATA_BYTES + 1],)
                .unwrap_err(),
            SseReplayError::EventDataTooLarge
        );
        assert_eq!(
            store
                .replay_sse_after(&session_id, retained[0].id())
                .unwrap(),
            vec![retained[1].clone()]
        );
    }

    #[test]
    fn termination_and_idle_expiry_remove_sse_replay_state() {
        let start = Instant::now();
        let store = McpSessionStore::with_limits(2, Duration::from_secs(10));
        let terminated = store.create_at(start).unwrap();
        let terminated_events = store
            .record_sse_response_at(&terminated, b"terminated".to_vec(), start)
            .unwrap();
        store.terminate_at(&terminated, start).unwrap();
        assert_eq!(
            store
                .replay_sse_after_at(&terminated, terminated_events[0].id(), start)
                .unwrap_err(),
            SseReplayError::SessionNotFound
        );

        let expired = store.create_at(start).unwrap();
        let expired_events = store
            .record_sse_response_at(&expired, b"expired".to_vec(), start)
            .unwrap();
        assert_eq!(
            store
                .replay_sse_after_at(
                    &expired,
                    expired_events[0].id(),
                    start + Duration::from_secs(10),
                )
                .unwrap_err(),
            SseReplayError::SessionNotFound
        );
    }
}

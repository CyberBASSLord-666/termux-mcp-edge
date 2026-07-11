//! Bounded in-memory lifecycle state for Streamable HTTP MCP sessions.
//!
//! Session records intentionally retain only a random identifier, lifecycle
//! phase, and last-activity timestamp. Client-provided initialization metadata
//! is validated by the transport but is never stored here.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use uuid::Uuid;

pub(crate) const MAX_MCP_SESSIONS: usize = 64;
pub(crate) const MCP_SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

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
            },
        );
        Ok(session_id)
    }

    fn phase_at(
        &self,
        session_id: &str,
        now: Instant,
    ) -> Result<SessionPhase, SessionStoreError> {
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

        assert_eq!(Uuid::parse_str(&session_id).unwrap().to_string(), session_id);
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
            store.create_at(start + Duration::from_secs(10)).unwrap_err(),
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
}

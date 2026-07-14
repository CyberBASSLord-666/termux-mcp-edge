//! Bounded in-memory lifecycle state for Streamable HTTP MCP sessions.
//!
//! Session records retain only a random identifier, lifecycle phase, last-activity
//! timestamp, and an optional server-derived authenticated-principal association.
//! Client-provided initialization metadata and raw credentials are never stored here.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use uuid::Uuid;

use crate::auth::AuthenticatedPrincipal;

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
    PrincipalMismatch,
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
    principal: Option<AuthenticatedPrincipal>,
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

    pub(crate) fn create(
        &self,
        principal: Option<&AuthenticatedPrincipal>,
    ) -> Result<String, SessionStoreError> {
        self.create_at(principal, Instant::now())
    }

    pub(crate) fn phase(
        &self,
        session_id: &str,
        principal: Option<&AuthenticatedPrincipal>,
    ) -> Result<SessionPhase, SessionStoreError> {
        self.phase_at(session_id, principal, Instant::now())
    }

    pub(crate) fn activate(
        &self,
        session_id: &str,
        principal: Option<&AuthenticatedPrincipal>,
    ) -> Result<(), SessionStoreError> {
        self.activate_at(session_id, principal, Instant::now())
    }

    pub(crate) fn terminate(
        &self,
        session_id: &str,
        principal: Option<&AuthenticatedPrincipal>,
    ) -> Result<(), SessionStoreError> {
        self.terminate_at(session_id, principal, Instant::now())
    }

    fn create_at(
        &self,
        principal: Option<&AuthenticatedPrincipal>,
        now: Instant,
    ) -> Result<String, SessionStoreError> {
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
                principal: principal.cloned(),
                last_activity: now,
            },
        );
        Ok(session_id)
    }

    fn phase_at(
        &self,
        session_id: &str,
        principal: Option<&AuthenticatedPrincipal>,
        now: Instant,
    ) -> Result<SessionPhase, SessionStoreError> {
        let mut registry = self.inner.lock().map_err(|_| SessionStoreError::Poisoned)?;
        registry.prune_expired(now, self.idle_timeout);
        let session = registry
            .sessions
            .get_mut(session_id)
            .ok_or(SessionStoreError::NotFound)?;
        validate_principal(session, principal)?;
        session.last_activity = now;
        Ok(session.phase)
    }

    fn activate_at(
        &self,
        session_id: &str,
        principal: Option<&AuthenticatedPrincipal>,
        now: Instant,
    ) -> Result<(), SessionStoreError> {
        let mut registry = self.inner.lock().map_err(|_| SessionStoreError::Poisoned)?;
        registry.prune_expired(now, self.idle_timeout);
        let session = registry
            .sessions
            .get_mut(session_id)
            .ok_or(SessionStoreError::NotFound)?;
        validate_principal(session, principal)?;
        session.phase = SessionPhase::Active;
        session.last_activity = now;
        Ok(())
    }

    fn terminate_at(
        &self,
        session_id: &str,
        principal: Option<&AuthenticatedPrincipal>,
        now: Instant,
    ) -> Result<(), SessionStoreError> {
        let mut registry = self.inner.lock().map_err(|_| SessionStoreError::Poisoned)?;
        registry.prune_expired(now, self.idle_timeout);
        let session = registry
            .sessions
            .get(session_id)
            .ok_or(SessionStoreError::NotFound)?;
        validate_principal(session, principal)?;
        registry.sessions.remove(session_id);
        Ok(())
    }
}

fn validate_principal(
    session: &SessionRecord,
    presented: Option<&AuthenticatedPrincipal>,
) -> Result<(), SessionStoreError> {
    match (session.principal.as_ref(), presented) {
        (None, None) => Ok(()),
        (Some(expected), Some(presented)) if expected == presented => Ok(()),
        _ => Err(SessionStoreError::PrincipalMismatch),
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
        let session_id = store.create(None).unwrap();

        assert_eq!(
            Uuid::parse_str(&session_id).unwrap().to_string(),
            session_id
        );
        assert!(session_id.bytes().all(|byte| (0x21..=0x7e).contains(&byte)));
        assert_eq!(
            store.phase(&session_id, None).unwrap(),
            SessionPhase::AwaitingInitialized
        );
    }

    #[test]
    fn activation_is_idempotent_and_scoped_to_one_session() {
        let store = McpSessionStore::new();
        let first = store.create(None).unwrap();
        let second = store.create(None).unwrap();

        store.activate(&first, None).unwrap();
        store.activate(&first, None).unwrap();

        assert_eq!(store.phase(&first, None).unwrap(), SessionPhase::Active);
        assert_eq!(
            store.phase(&second, None).unwrap(),
            SessionPhase::AwaitingInitialized
        );
    }

    #[test]
    fn principal_bound_sessions_reject_missing_and_cross_principal_access() {
        let store = McpSessionStore::new();
        let first = AuthenticatedPrincipal::configured("operator.primary:v1").unwrap();
        let second = AuthenticatedPrincipal::configured("operator.secondary:v1").unwrap();
        let session_id = store.create(Some(&first)).unwrap();

        assert_eq!(
            store.phase(&session_id, Some(&first)).unwrap(),
            SessionPhase::AwaitingInitialized
        );
        assert_eq!(
            store.phase(&session_id, None).unwrap_err(),
            SessionStoreError::PrincipalMismatch
        );
        assert_eq!(
            store.phase(&session_id, Some(&second)).unwrap_err(),
            SessionStoreError::PrincipalMismatch
        );
        assert_eq!(
            store.activate(&session_id, Some(&second)).unwrap_err(),
            SessionStoreError::PrincipalMismatch
        );
        assert_eq!(
            store.terminate(&session_id, None).unwrap_err(),
            SessionStoreError::PrincipalMismatch
        );

        store.activate(&session_id, Some(&first)).unwrap();
        assert_eq!(
            store.phase(&session_id, Some(&first)).unwrap(),
            SessionPhase::Active
        );
        store.terminate(&session_id, Some(&first)).unwrap();
    }

    #[test]
    fn unbound_sessions_reject_later_principal_injection() {
        let store = McpSessionStore::new();
        let principal = AuthenticatedPrincipal::configured("operator.primary:v1").unwrap();
        let session_id = store.create(None).unwrap();

        assert_eq!(
            store.phase(&session_id, Some(&principal)).unwrap_err(),
            SessionStoreError::PrincipalMismatch
        );
        assert_eq!(
            store.phase(&session_id, None).unwrap(),
            SessionPhase::AwaitingInitialized
        );
    }

    #[test]
    fn capacity_is_bounded_until_a_session_is_terminated() {
        let store = McpSessionStore::with_limits(2, Duration::from_secs(60));
        let first = store.create(None).unwrap();
        let _second = store.create(None).unwrap();

        assert_eq!(
            store.create(None).unwrap_err(),
            SessionStoreError::CapacityExhausted
        );

        store.terminate(&first, None).unwrap();
        assert!(store.create(None).is_ok());
    }

    #[test]
    fn idle_sessions_expire_and_release_capacity() {
        let start = Instant::now();
        let store = McpSessionStore::with_limits(1, Duration::from_secs(10));
        let expired = store.create_at(None, start).unwrap();

        assert_eq!(
            store
                .phase_at(&expired, None, start + Duration::from_secs(9))
                .unwrap(),
            SessionPhase::AwaitingInitialized
        );
        assert_eq!(
            store
                .create_at(None, start + Duration::from_secs(10))
                .unwrap_err(),
            SessionStoreError::CapacityExhausted
        );

        let replacement = store
            .create_at(None, start + Duration::from_secs(20))
            .unwrap();
        assert_ne!(replacement, expired);
        assert_eq!(
            store
                .phase_at(&expired, None, start + Duration::from_secs(20))
                .unwrap_err(),
            SessionStoreError::NotFound
        );
    }

    #[test]
    fn terminated_and_unknown_sessions_are_not_found() {
        let store = McpSessionStore::new();
        let session_id = store.create(None).unwrap();

        store.terminate(&session_id, None).unwrap();

        assert_eq!(
            store.phase(&session_id, None).unwrap_err(),
            SessionStoreError::NotFound
        );
        assert_eq!(
            store.terminate("not-a-session", None).unwrap_err(),
            SessionStoreError::NotFound
        );
    }
}

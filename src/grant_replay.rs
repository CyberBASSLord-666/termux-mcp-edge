//! Process-global replay protection shared by equivalent grant authorities.
//!
//! The registry deliberately retains strong references for the process
//! lifetime. Reconstructing an authority therefore cannot forget a consumed
//! grant or reset its rollback sentinel. Registry namespaces are opaque keyed
//! digests and are never exposed through public APIs or debug output.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, OnceLock},
};

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::request_grant_capability::RequestGrantCapability;

type HmacSha256 = Hmac<Sha256>;

const GRANT_ID_BYTES: usize = 16;
const NAMESPACE_BYTES: usize = 32;
const NAMESPACE_DOMAIN: &[u8] = b"termux-mcp:grant-replay-namespace:v1\0";

/// Bounds retained authority namespaces. Per-family authorities separately
/// bound the replay entries within each namespace. Existing namespaces remain
/// available when the registry is full; only a previously unseen authority
/// equivalence class is rejected.
const MAX_SHARED_REPLAY_NAMESPACES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedReplayError {
    Expired,
    FutureIssued,
    LifetimeExceeded,
    Replayed,
    ClockRollback,
    CapacityExhausted,
    StateUnavailable,
}

#[derive(Clone)]
pub(crate) struct SharedReplayState {
    inner: Arc<Mutex<ReplayState>>,
}

struct ReplayState {
    consumed: BTreeMap<[u8; GRANT_ID_BYTES], u64>,
    last_observed_unix_seconds: Option<u64>,
    capacity: usize,
}

impl SharedReplayState {
    fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ReplayState {
                consumed: BTreeMap::new(),
                last_observed_unix_seconds: None,
                capacity,
            })),
        }
    }

    /// Atomically applies clock monotonicity, expiry pruning, replay detection,
    /// capacity enforcement, and grant consumption to one shared namespace.
    pub(crate) fn consume(
        &self,
        grant_id: [u8; GRANT_ID_BYTES],
        issued_unix_seconds: u64,
        expires_unix_seconds: u64,
        now_unix_seconds: u64,
        maximum_lifetime_seconds: u64,
        maximum_future_skew_seconds: u64,
    ) -> Result<(), SharedReplayError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| SharedReplayError::StateUnavailable)?;
        if state
            .last_observed_unix_seconds
            .is_some_and(|last| now_unix_seconds < last)
        {
            return Err(SharedReplayError::ClockRollback);
        }
        state.last_observed_unix_seconds = Some(now_unix_seconds);

        let lifetime = expires_unix_seconds
            .checked_sub(issued_unix_seconds)
            .ok_or(SharedReplayError::LifetimeExceeded)?;
        if lifetime == 0 || lifetime > maximum_lifetime_seconds {
            return Err(SharedReplayError::LifetimeExceeded);
        }
        if issued_unix_seconds > now_unix_seconds.saturating_add(maximum_future_skew_seconds) {
            return Err(SharedReplayError::FutureIssued);
        }
        if now_unix_seconds >= expires_unix_seconds {
            return Err(SharedReplayError::Expired);
        }

        state
            .consumed
            .retain(|_, expiry| *expiry > now_unix_seconds);
        if state.consumed.contains_key(&grant_id) {
            return Err(SharedReplayError::Replayed);
        }
        if state.consumed.len() >= state.capacity {
            return Err(SharedReplayError::CapacityExhausted);
        }
        state.consumed.insert(grant_id, expires_unix_seconds);
        Ok(())
    }

    /// Capacity is part of the retained namespace configuration. Equivalent
    /// authorities must agree exactly rather than changing each other's limit.
    fn require_capacity(&self, requested_capacity: usize) -> Result<(), SharedReplayError> {
        let state = self
            .inner
            .lock()
            .map_err(|_| SharedReplayError::StateUnavailable)?;
        if state.capacity == requested_capacity {
            Ok(())
        } else {
            Err(SharedReplayError::StateUnavailable)
        }
    }

    #[cfg(test)]
    pub(crate) fn poison_for_test(&self) {
        let inner = Arc::clone(&self.inner);
        let _ = std::thread::spawn(move || {
            let _guard = inner
                .lock()
                .expect("test replay state must start available");
            panic!("poison test replay lock");
        })
        .join();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ReplayNamespace([u8; NAMESPACE_BYTES]);

struct ReplayRegistry {
    states: HashMap<ReplayNamespace, SharedReplayState>,
    namespace_capacity: usize,
}

impl ReplayRegistry {
    fn new(namespace_capacity: usize) -> Self {
        Self {
            states: HashMap::new(),
            namespace_capacity,
        }
    }

    fn resolve(
        &mut self,
        namespace: ReplayNamespace,
        replay_capacity: usize,
    ) -> Result<SharedReplayState, SharedReplayError> {
        if let Some(state) = self.states.get(&namespace) {
            state.require_capacity(replay_capacity)?;
            return Ok(state.clone());
        }
        if self.states.len() >= self.namespace_capacity {
            return Err(SharedReplayError::StateUnavailable);
        }
        let state = SharedReplayState::new(replay_capacity);
        self.states.insert(namespace, state.clone());
        Ok(state)
    }
}

static SHARED_REPLAY_REGISTRY: OnceLock<Mutex<ReplayRegistry>> = OnceLock::new();

/// Resolves the process-global replay state for one exact effective authority.
///
/// The HMAC key contributes both as the namespace derivation key and through
/// the independently keyed principal binding. As a result, family, exact key
/// ID, HMAC key, and principal must all be equivalent to share state. The
/// resulting namespace is private, opaque, and never formatted or returned.
pub(crate) fn shared_replay_state(
    capability: RequestGrantCapability,
    key_id: &str,
    key: &[u8],
    principal_digest: &[u8; NAMESPACE_BYTES],
    replay_capacity: usize,
) -> Result<SharedReplayState, SharedReplayError> {
    if replay_capacity == 0 {
        return Err(SharedReplayError::StateUnavailable);
    }
    let namespace = replay_namespace(capability, key_id, key, principal_digest)?;
    let registry = SHARED_REPLAY_REGISTRY
        .get_or_init(|| Mutex::new(ReplayRegistry::new(MAX_SHARED_REPLAY_NAMESPACES)));
    resolve_locked(registry, namespace, replay_capacity)
}

fn resolve_locked(
    registry: &Mutex<ReplayRegistry>,
    namespace: ReplayNamespace,
    replay_capacity: usize,
) -> Result<SharedReplayState, SharedReplayError> {
    registry
        .lock()
        .map_err(|_| SharedReplayError::StateUnavailable)?
        .resolve(namespace, replay_capacity)
}

fn replay_namespace(
    capability: RequestGrantCapability,
    key_id: &str,
    key: &[u8],
    principal_digest: &[u8; NAMESPACE_BYTES],
) -> Result<ReplayNamespace, SharedReplayError> {
    let key_id_length =
        u64::try_from(key_id.len()).map_err(|_| SharedReplayError::StateUnavailable)?;
    let mut namespace =
        HmacSha256::new_from_slice(key).map_err(|_| SharedReplayError::StateUnavailable)?;
    namespace.update(NAMESPACE_DOMAIN);
    namespace.update(&[capability.wire_code()]);
    namespace.update(&key_id_length.to_be_bytes());
    namespace.update(key_id.as_bytes());
    namespace.update(principal_digest);
    Ok(ReplayNamespace(namespace.finalize().into_bytes().into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn namespace(byte: u8) -> ReplayNamespace {
        ReplayNamespace([byte; NAMESPACE_BYTES])
    }

    #[test]
    fn bounded_registry_keeps_existing_namespaces_available() {
        let mut registry = ReplayRegistry::new(2);
        let first = registry.resolve(namespace(1), 2).unwrap();
        registry.resolve(namespace(2), 2).unwrap();
        assert!(matches!(
            registry.resolve(namespace(3), 2),
            Err(SharedReplayError::StateUnavailable)
        ));

        let same_first = registry.resolve(namespace(1), 2).unwrap();
        first
            .consume([7; GRANT_ID_BYTES], 1, 100, 1, 120, 5)
            .unwrap();
        assert_eq!(
            same_first.consume([7; GRANT_ID_BYTES], 1, 100, 1, 120, 5),
            Err(SharedReplayError::Replayed)
        );
    }

    #[test]
    fn registry_holds_state_strongly_after_callers_drop_handles() {
        let mut registry = ReplayRegistry::new(1);
        let state = registry.resolve(namespace(1), 2).unwrap();
        state
            .consume([7; GRANT_ID_BYTES], 1, 100, 1, 120, 5)
            .unwrap();
        drop(state);

        let reconstructed = registry.resolve(namespace(1), 2).unwrap();
        assert_eq!(
            reconstructed.consume([7; GRANT_ID_BYTES], 1, 100, 1, 120, 5),
            Err(SharedReplayError::Replayed)
        );
    }

    #[test]
    fn equivalent_resolution_requires_one_shared_capacity() {
        let mut registry = ReplayRegistry::new(1);
        let original = registry.resolve(namespace(1), 4).unwrap();
        assert!(matches!(
            registry.resolve(namespace(1), 1),
            Err(SharedReplayError::StateUnavailable)
        ));
        assert!(matches!(
            registry.resolve(namespace(1), 8),
            Err(SharedReplayError::StateUnavailable)
        ));
        registry.resolve(namespace(1), 4).unwrap();
        original
            .consume([1; GRANT_ID_BYTES], 1, 100, 1, 120, 5)
            .unwrap();
    }

    #[test]
    fn authority_equivalence_dimensions_produce_isolated_namespaces() {
        let principal = [9; NAMESPACE_BYTES];
        let baseline = replay_namespace(
            RequestGrantCapability::CreateDirectory,
            "primary-1",
            &[1; 32],
            &principal,
        )
        .unwrap();
        let isolated = [
            replay_namespace(
                RequestGrantCapability::WriteFile,
                "primary-1",
                &[1; 32],
                &principal,
            )
            .unwrap(),
            replay_namespace(
                RequestGrantCapability::CreateDirectory,
                "primary-2",
                &[1; 32],
                &principal,
            )
            .unwrap(),
            replay_namespace(
                RequestGrantCapability::CreateDirectory,
                "primary-1",
                &[2; 32],
                &principal,
            )
            .unwrap(),
            replay_namespace(
                RequestGrantCapability::CreateDirectory,
                "primary-1",
                &[1; 32],
                &[8; NAMESPACE_BYTES],
            )
            .unwrap(),
        ];
        let mut registry = ReplayRegistry::new(isolated.len() + 1);
        let baseline_state = registry.resolve(baseline, 1).unwrap();
        baseline_state
            .consume([7; GRANT_ID_BYTES], 1, 100, 1, 120, 5)
            .unwrap();
        for isolated_namespace in isolated {
            assert_ne!(baseline.0, isolated_namespace.0);
            registry
                .resolve(isolated_namespace, 1)
                .unwrap()
                .consume([7; GRANT_ID_BYTES], 1, 100, 1, 120, 5)
                .unwrap();
        }
    }

    #[test]
    fn poisoned_registry_lock_fails_closed_without_touching_the_global_registry() {
        let registry = Arc::new(Mutex::new(ReplayRegistry::new(1)));
        let poison_target = Arc::clone(&registry);
        let _ = std::thread::spawn(move || {
            let _guard = poison_target
                .lock()
                .expect("test registry must start available");
            panic!("poison test registry lock");
        })
        .join();
        assert!(matches!(
            resolve_locked(&registry, namespace(1), 1),
            Err(SharedReplayError::StateUnavailable)
        ));
    }
}

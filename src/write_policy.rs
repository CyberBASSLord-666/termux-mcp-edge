//! Default-deny write policy helpers for staged MCP filesystem write support.
//!
//! The MCP transport must remain dry-run-first. These helpers keep that intent
//! centralized before the write-capable transport surface is exposed.

pub const DEFAULT_MAX_WRITE_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    DryRun,
    Mutating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WritePolicy {
    max_write_bytes: usize,
}

impl Default for WritePolicy {
    fn default() -> Self {
        Self {
            max_write_bytes: DEFAULT_MAX_WRITE_BYTES,
        }
    }
}

impl WritePolicy {
    pub const fn new(max_write_bytes: usize) -> Self {
        Self { max_write_bytes }
    }

    pub const fn max_write_bytes(self) -> usize {
        self.max_write_bytes
    }

    pub fn resolve_mode(self, dry_run: Option<bool>) -> WriteMode {
        if dry_run.unwrap_or(true) {
            WriteMode::DryRun
        } else {
            WriteMode::Mutating
        }
    }

    pub fn validate_payload_size(self, bytes: usize) -> Result<(), WritePolicyError> {
        if bytes > self.max_write_bytes {
            Err(WritePolicyError::PayloadTooLarge {
                bytes,
                max_bytes: self.max_write_bytes,
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePolicyError {
    PayloadTooLarge { bytes: usize, max_bytes: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_mode_defaults_to_dry_run_when_omitted() {
        let policy = WritePolicy::default();

        assert_eq!(policy.resolve_mode(None), WriteMode::DryRun);
        assert_eq!(policy.resolve_mode(Some(true)), WriteMode::DryRun);
    }

    #[test]
    fn write_mode_requires_explicit_false_for_mutation() {
        let policy = WritePolicy::default();

        assert_eq!(policy.resolve_mode(Some(false)), WriteMode::Mutating);
    }

    #[test]
    fn write_payload_size_allows_exact_limit() {
        let policy = WritePolicy::new(16);

        assert_eq!(policy.validate_payload_size(16), Ok(()));
    }

    #[test]
    fn write_payload_size_rejects_above_limit() {
        let policy = WritePolicy::new(16);

        assert_eq!(
            policy.validate_payload_size(17),
            Err(WritePolicyError::PayloadTooLarge {
                bytes: 17,
                max_bytes: 16,
            })
        );
    }
}

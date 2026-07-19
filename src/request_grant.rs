//! Shared wire invariants for live request-scoped mutation grants.
//!
//! Grant payloads, binding domains, replay stores, and validators remain
//! deliberately family-specific. This module owns only the transport-wide
//! header contract and the signed capability-code namespace so independent
//! authorities cannot accidentally reuse one family discriminator.

pub(crate) const REQUEST_GRANT_HEADER: &str = "mcp-capability-grant";
pub(crate) const MAX_REQUEST_GRANT_HEADER_BYTES: usize = 384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RequestGrantCapability {
    CreateDirectory = 1,
    WriteFile = 2,
    AndroidVolumeControl = 3,
    CopyFile = 4,
}

impl RequestGrantCapability {
    pub(crate) const fn code(self) -> u8 {
        self as u8
    }
}

// Construct every reserved discriminator in non-test builds. Duplicate enum
// discriminants are rejected by the compiler, including the reserved copy
// capability before its authority is implemented.
const _: [u8; 4] = [
    RequestGrantCapability::CreateDirectory.code(),
    RequestGrantCapability::WriteFile.code(),
    RequestGrantCapability::AndroidVolumeControl.code(),
    RequestGrantCapability::CopyFile.code(),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_grant_registry_is_stable_and_unique() {
        let codes = [
            RequestGrantCapability::CreateDirectory.code(),
            RequestGrantCapability::WriteFile.code(),
            RequestGrantCapability::AndroidVolumeControl.code(),
            RequestGrantCapability::CopyFile.code(),
        ];

        assert_eq!(codes, [1, 2, 3, 4]);
        for (index, code) in codes.iter().enumerate() {
            assert!(!codes[..index].contains(code));
        }
        assert_eq!(REQUEST_GRANT_HEADER, "mcp-capability-grant");
        assert_eq!(MAX_REQUEST_GRANT_HEADER_BYTES, 384);
    }
}

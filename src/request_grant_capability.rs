//! Globally unique wire identifiers for request-grant capability families.
//!
//! These values are part of the signed grant binding. Keep every live and
//! reserved request-grant family in this one registry so independently
//! developed grant modules cannot silently reuse a wire identifier.

/// Canonical header used by every live request-grant family.
pub(crate) const REQUEST_GRANT_HEADER: &str = "mcp-capability-grant";
/// Maximum ASCII bytes admitted for any request-grant token.
///
/// The longest live format is the create-directory token: a 260-hex-character
/// payload plus version, separators, the bounded key identifier, and MAC fits
/// within this fixed service-wide ceiling.
pub(crate) const MAX_REQUEST_GRANT_HEADER_BYTES: usize = 384;

/// One globally allocated request-grant capability family.
///
/// The discriminants are wire compatibility commitments. Never renumber or
/// reuse an allocated family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RequestGrantCapability {
    CreateDirectory = 1,
    WriteFile = 2,
    AndroidVolume = 3,
    CopyFile = 4,
}

impl RequestGrantCapability {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 4] = [
        Self::CreateDirectory,
        Self::WriteFile,
        Self::AndroidVolume,
        Self::CopyFile,
    ];

    pub(crate) const fn wire_code(self) -> u8 {
        self as u8
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn capability_registry_preserves_exact_unique_wire_codes() {
        assert_eq!(REQUEST_GRANT_HEADER, "mcp-capability-grant");
        assert_eq!(MAX_REQUEST_GRANT_HEADER_BYTES, 384);
        assert_eq!(RequestGrantCapability::CreateDirectory.wire_code(), 1);
        assert_eq!(RequestGrantCapability::WriteFile.wire_code(), 2);
        assert_eq!(RequestGrantCapability::AndroidVolume.wire_code(), 3);
        assert_eq!(RequestGrantCapability::CopyFile.wire_code(), 4);
        assert_eq!(
            RequestGrantCapability::ALL.map(RequestGrantCapability::wire_code),
            [1, 2, 3, 4]
        );

        let unique = RequestGrantCapability::ALL
            .into_iter()
            .map(RequestGrantCapability::wire_code)
            .collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), RequestGrantCapability::ALL.len());
    }

    #[cfg(feature = "android-volume-control")]
    mod cross_family {
        use sha2::{Digest, Sha256};

        use crate::{
            android_volume_control::AndroidVolumeStreamName,
            android_volume_grant::{
                AndroidVolumeGrantAuthority, AndroidVolumeGrantError, AndroidVolumeGrantTarget,
            },
            copy_file_grant::{
                CopyFileGrantAuthority, CopyFileGrantError, CopyFileGrantTarget,
                CopyFileSourceIdentity,
            },
            create_directory_grant::{
                CreateDirectoryGrantAuthority, CreateDirectoryGrantError,
                CreateDirectoryGrantTarget,
            },
            write_file_grant::{
                WriteFileDisposition, WriteFileGrantAuthority, WriteFileGrantError,
                WriteFileGrantTarget,
            },
        };

        const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        const PRINCIPAL: &str = "same-static-principal";
        const SESSION: &str = "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee";
        const NOW: u64 = 1_725_000_000;

        fn assert_private_rejection(expected: &str, reason: &str, rendered: &str) {
            assert_eq!(reason, expected);
            assert_eq!(rendered, reason);
            for private in [
                KEY,
                PRINCIPAL,
                SESSION,
                "projects",
                "payload",
                "source.bin",
                "destination.bin",
            ] {
                assert!(!rendered.contains(private));
            }
        }

        #[test]
        fn every_live_grant_family_rejects_every_other_family_without_consumption() {
            let create =
                CreateDirectoryGrantAuthority::from_hex_key("primary-1", KEY, PRINCIPAL).unwrap();
            let volume =
                AndroidVolumeGrantAuthority::from_hex_key("primary-1", KEY, PRINCIPAL).unwrap();
            let write = WriteFileGrantAuthority::from_hex_key("primary-1", KEY, PRINCIPAL).unwrap();
            let copy = CopyFileGrantAuthority::from_hex_key("primary-1", KEY, PRINCIPAL).unwrap();

            let create_target = CreateDirectoryGrantTarget::from_normalized_components(
                41,
                73,
                [b"projects".as_slice(), b"new-directory".as_slice()],
            )
            .unwrap();
            let volume_target =
                AndroidVolumeGrantTarget::new(AndroidVolumeStreamName::Music, 9).unwrap();
            let write_target = WriteFileGrantTarget::from_normalized_components(
                41,
                73,
                [b"projects".as_slice(), b"settings.json".as_slice()],
                b"payload",
                WriteFileDisposition::Create,
                None,
            )
            .unwrap();
            let copy_target = CopyFileGrantTarget::from_normalized_components(
                41,
                73,
                [b"projects".as_slice(), b"source.bin".as_slice()],
                CopyFileSourceIdentity::new(101, 202, 7, 1_700_000_000, 123_456_789, 1).unwrap(),
                Sha256::digest(b"payload").into(),
                43,
                79,
                [b"archive".as_slice(), b"destination.bin".as_slice()],
            )
            .unwrap();

            let create_token = create.issue_at(SESSION, &create_target, NOW).unwrap();
            let volume_token = volume.issue_at(SESSION, volume_target, NOW).unwrap();
            let write_token = write.issue_at(SESSION, &write_target, NOW).unwrap();
            let copy_token = copy.issue_at(SESSION, &copy_target, NOW).unwrap();

            for error in [
                create
                    .consume_at(Some(&volume_token), SESSION, &create_target, NOW + 1)
                    .unwrap_err(),
                create
                    .consume_at(Some(&write_token), SESSION, &create_target, NOW + 1)
                    .unwrap_err(),
                create
                    .consume_at(Some(&copy_token), SESSION, &create_target, NOW + 1)
                    .unwrap_err(),
            ] {
                assert_eq!(error, CreateDirectoryGrantError::Malformed);
                assert_private_rejection(
                    "capability_grant_malformed",
                    error.reason_code(),
                    &error.to_string(),
                );
            }
            for error in [
                volume
                    .consume_at(Some(&create_token), SESSION, volume_target, NOW + 1)
                    .unwrap_err(),
                volume
                    .consume_at(Some(&write_token), SESSION, volume_target, NOW + 1)
                    .unwrap_err(),
                volume
                    .consume_at(Some(&copy_token), SESSION, volume_target, NOW + 1)
                    .unwrap_err(),
            ] {
                assert_eq!(error, AndroidVolumeGrantError::Malformed);
                assert_private_rejection(
                    "capability_grant_malformed",
                    error.reason_code(),
                    &error.to_string(),
                );
            }
            for error in [
                write
                    .consume_at(Some(&create_token), SESSION, &write_target, NOW + 1)
                    .unwrap_err(),
                write
                    .consume_at(Some(&volume_token), SESSION, &write_target, NOW + 1)
                    .unwrap_err(),
            ] {
                assert_eq!(error, WriteFileGrantError::Malformed);
                assert_private_rejection(
                    "capability_grant_malformed",
                    error.reason_code(),
                    &error.to_string(),
                );
            }
            let write_rejects_copy = write
                .consume_at(Some(&copy_token), SESSION, &write_target, NOW + 1)
                .unwrap_err();
            assert_eq!(write_rejects_copy, WriteFileGrantError::BindingMismatch);
            assert_private_rejection(
                "capability_grant_binding_mismatch",
                write_rejects_copy.reason_code(),
                &write_rejects_copy.to_string(),
            );

            for error in [
                copy.consume_at(Some(&create_token), SESSION, &copy_target, NOW + 1)
                    .unwrap_err(),
                copy.consume_at(Some(&volume_token), SESSION, &copy_target, NOW + 1)
                    .unwrap_err(),
            ] {
                assert_eq!(error, CopyFileGrantError::Malformed);
                assert_private_rejection(
                    "capability_grant_malformed",
                    error.reason_code(),
                    &error.to_string(),
                );
            }
            let copy_rejects_write = copy
                .consume_at(Some(&write_token), SESSION, &copy_target, NOW + 1)
                .unwrap_err();
            assert_eq!(copy_rejects_write, CopyFileGrantError::BindingMismatch);
            assert_private_rejection(
                "capability_grant_binding_mismatch",
                copy_rejects_write.reason_code(),
                &copy_rejects_write.to_string(),
            );

            // Wrong-family attempts must not consume the source grant. Each
            // exact family can still accept its token once, then rejects only
            // the true same-family replay.
            create
                .consume_at(Some(&create_token), SESSION, &create_target, NOW + 1)
                .unwrap();
            volume
                .consume_at(Some(&volume_token), SESSION, volume_target, NOW + 1)
                .unwrap();
            write
                .consume_at(Some(&write_token), SESSION, &write_target, NOW + 1)
                .unwrap();
            copy.consume_at(Some(&copy_token), SESSION, &copy_target, NOW + 1)
                .unwrap();

            assert_eq!(
                create
                    .consume_at(Some(&create_token), SESSION, &create_target, NOW + 1)
                    .unwrap_err(),
                CreateDirectoryGrantError::Replayed
            );
            assert_eq!(
                volume
                    .consume_at(Some(&volume_token), SESSION, volume_target, NOW + 1)
                    .unwrap_err(),
                AndroidVolumeGrantError::Replayed
            );
            assert_eq!(
                write
                    .consume_at(Some(&write_token), SESSION, &write_target, NOW + 1)
                    .unwrap_err(),
                WriteFileGrantError::Replayed
            );
            assert_eq!(
                copy.consume_at(Some(&copy_token), SESSION, &copy_target, NOW + 1)
                    .unwrap_err(),
                CopyFileGrantError::Replayed
            );
        }
    }
}

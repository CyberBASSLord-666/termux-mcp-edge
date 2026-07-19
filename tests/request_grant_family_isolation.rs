#![cfg(feature = "mcp-runtime")]

use termux_mcp_server::{
    create_directory_grant::{CreateDirectoryGrantAuthority, CreateDirectoryGrantError},
    tools::FileSystemTools,
    write_file_grant::{content_sha256, WriteFileGrantAuthority, WriteFileGrantError},
};

#[cfg(feature = "android-volume-control")]
use termux_mcp_server::{
    android_volume_control::AndroidVolumeStreamName,
    android_volume_grant::{
        AndroidVolumeGrantAuthority, AndroidVolumeGrantError, AndroidVolumeGrantTarget,
    },
};

const KEY_ID: &str = "family-isolation-1";
const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const PRINCIPAL: &str = "shared-private-principal";
const SESSION: &str = "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee";
const NOW: u64 = 1_725_000_000;

#[test]
fn filesystem_grants_are_pairwise_isolated_and_wrong_use_does_not_consume() {
    let root = tempfile::tempdir().unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    let directory_target = tools
        .create_directory_grant_target(
            root.path()
                .join("isolated-directory")
                .to_string_lossy()
                .as_ref(),
        )
        .unwrap();
    let write_target = tools
        .write_file_grant_target(
            root.path()
                .join("isolated-file.txt")
                .to_string_lossy()
                .as_ref(),
            content_sha256(b"isolated content"),
        )
        .unwrap();
    let directory =
        CreateDirectoryGrantAuthority::from_hex_key(KEY_ID, KEY, PRINCIPAL).unwrap();
    let write = WriteFileGrantAuthority::from_hex_key(KEY_ID, KEY, PRINCIPAL).unwrap();
    let directory_grant = directory.issue_at(SESSION, &directory_target, NOW).unwrap();
    let write_grant = write.issue_at(SESSION, &write_target, NOW).unwrap();

    assert_eq!(
        directory
            .consume_at(Some(&write_grant), SESSION, &directory_target, NOW)
            .unwrap_err(),
        CreateDirectoryGrantError::BindingMismatch
    );
    assert_eq!(
        write
            .consume_at(Some(&directory_grant), SESSION, &write_target, NOW)
            .unwrap_err(),
        WriteFileGrantError::BindingMismatch
    );

    directory
        .consume_at(Some(&directory_grant), SESSION, &directory_target, NOW)
        .unwrap();
    write
        .consume_at(Some(&write_grant), SESSION, &write_target, NOW)
        .unwrap();
}

#[cfg(feature = "android-volume-control")]
#[test]
fn all_live_grant_families_reject_every_wrong_family_before_consumption() {
    let root = tempfile::tempdir().unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    let directory_target = tools
        .create_directory_grant_target(
            root.path()
                .join("all-family-directory")
                .to_string_lossy()
                .as_ref(),
        )
        .unwrap();
    let write_target = tools
        .write_file_grant_target(
            root.path()
                .join("all-family-file.txt")
                .to_string_lossy()
                .as_ref(),
            content_sha256(b"all-family content"),
        )
        .unwrap();
    let volume_target =
        AndroidVolumeGrantTarget::new(AndroidVolumeStreamName::Music, 9).unwrap();
    let directory =
        CreateDirectoryGrantAuthority::from_hex_key(KEY_ID, KEY, PRINCIPAL).unwrap();
    let write = WriteFileGrantAuthority::from_hex_key(KEY_ID, KEY, PRINCIPAL).unwrap();
    let volume = AndroidVolumeGrantAuthority::from_hex_key(KEY_ID, KEY, PRINCIPAL).unwrap();
    let directory_grant = directory.issue_at(SESSION, &directory_target, NOW).unwrap();
    let write_grant = write.issue_at(SESSION, &write_target, NOW).unwrap();
    let volume_grant = volume.issue_at(SESSION, volume_target, NOW).unwrap();

    assert!(directory
        .consume_at(Some(&write_grant), SESSION, &directory_target, NOW)
        .is_err());
    assert!(directory
        .consume_at(Some(&volume_grant), SESSION, &directory_target, NOW)
        .is_err());
    assert!(write
        .consume_at(Some(&directory_grant), SESSION, &write_target, NOW)
        .is_err());
    assert!(write
        .consume_at(Some(&volume_grant), SESSION, &write_target, NOW)
        .is_err());
    assert!(volume
        .consume_at(Some(&directory_grant), SESSION, volume_target, NOW)
        .is_err());
    assert!(volume
        .consume_at(Some(&write_grant), SESSION, volume_target, NOW)
        .is_err());

    directory
        .consume_at(Some(&directory_grant), SESSION, &directory_target, NOW)
        .unwrap();
    write
        .consume_at(Some(&write_grant), SESSION, &write_target, NOW)
        .unwrap();
    volume
        .consume_at(Some(&volume_grant), SESSION, volume_target, NOW)
        .unwrap();

    assert_eq!(
        directory
            .consume_at(Some(&directory_grant), SESSION, &directory_target, NOW)
            .unwrap_err(),
        CreateDirectoryGrantError::Replayed
    );
    assert_eq!(
        write
            .consume_at(Some(&write_grant), SESSION, &write_target, NOW)
            .unwrap_err(),
        WriteFileGrantError::Replayed
    );
    assert_eq!(
        volume
            .consume_at(Some(&volume_grant), SESSION, volume_target, NOW)
            .unwrap_err(),
        AndroidVolumeGrantError::Replayed
    );
}

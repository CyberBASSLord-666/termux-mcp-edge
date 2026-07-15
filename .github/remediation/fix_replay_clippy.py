#!/usr/bin/env python3
from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one match, found {count}: {old!r}")
    path.write_text(text.replace(old, new, 1))


path = Path("src/directory_replay.rs")
replace_once(
    path,
    """struct ReplayKeyring {\n    active_key_id: String,\n    keys: BTreeMap<String, ReplayKey>,\n    fingerprints: BTreeSet<[u8; DIGEST_BYTES]>,\n}\n""",
    """struct ReplayKeyring {\n    active_key_id: String,\n    keys: BTreeMap<String, ReplayKey>,\n}\n""",
)
replace_once(
    path,
    """struct ParsedRecord {\n    raw: [u8; REPLAY_RECORD_BYTES],\n    kind: u8,\n    grant_version: u8,\n    replay_key_id: String,\n    verification_key_id: String,\n    retention_until_unix_seconds: u64,\n    observed_unix_seconds: u64,\n    digest: [u8; DIGEST_BYTES],\n}\n""",
    """struct ParsedRecord {\n    raw: [u8; REPLAY_RECORD_BYTES],\n    kind: u8,\n    replay_key_id: String,\n    retention_until_unix_seconds: u64,\n    observed_unix_seconds: u64,\n    digest: [u8; DIGEST_BYTES],\n}\n""",
)
replace_once(
    path,
    """        Ok(ParsedRecord {\n            raw,\n            kind,\n            grant_version,\n            replay_key_id,\n            verification_key_id,\n            retention_until_unix_seconds,\n            observed_unix_seconds,\n            digest,\n        })\n""",
    """        Ok(ParsedRecord {\n            raw,\n            kind,\n            replay_key_id,\n            retention_until_unix_seconds,\n            observed_unix_seconds,\n            digest,\n        })\n""",
)
replace_once(
    path,
    """    Ok(ReplayKeyring {\n        active_key_id: raw.active_kid,\n        keys,\n        fingerprints,\n    })\n""",
    """    Ok(ReplayKeyring {\n        active_key_id: raw.active_kid,\n        keys,\n    })\n""",
)

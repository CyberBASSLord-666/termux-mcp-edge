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
    """    fn replay_config(directory: &Path, keyring_path: PathBuf) -> DirectoryGrantConfig {\n        DirectoryGrantConfig {\n""",
    """    fn replay_config(directory: &Path, keyring_path: PathBuf) -> DirectoryGrantConfig {\n        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();\n        DirectoryGrantConfig {\n""",
)

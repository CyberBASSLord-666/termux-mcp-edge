#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077
set +x

usage() {
  cat <<'EOF'
Usage: verify_runtime_snapshot.sh \
  --archive /absolute/termux-qualified-runtime-image-v1.tar.gz \
  --package-lock /absolute/termux-runtime-package-lock-v1.json \
  --snapshot-manifest /absolute/termux-runtime-snapshot-v1.json \
  --aggregate-evidence /absolute/termux-native-aggregate-evidence-v4.json \
  --deployment-evidence /absolute/automated-native-deployment-v1.json \
  --output /absolute/private/termux-runtime-snapshot-replay-v1.json

Loads one retained Docker image archive on a native ARM64 host, executes it by
its exact config/image ID with networking disabled, and atomically writes one
closed non-authoritative replay report. It never builds an image or resolves,
downloads, or installs packages.
EOF
}

if (($# == 1)) && [[ "$1" == -h || "$1" == --help ]]; then
  usage
  exit 0
fi

for command_name in python3 docker; do
  command -v "$command_name" >/dev/null 2>&1 || {
    printf '[runtime-snapshot-replay] ERROR: required command missing: %s\n' \
      "$command_name" >&2
    exit 1
  }
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
COMMIT_HELPER="$SCRIPT_DIR/commit_verified_file.py"
[[ -f "$COMMIT_HELPER" && ! -L "$COMMIT_HELPER" ]] || {
  printf '[runtime-snapshot-replay] ERROR: verified-file commit helper missing\n' >&2
  exit 1
}

python3 - "$COMMIT_HELPER" "$@" <<'PY'
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import math
import os
import pathlib
import posixpath
import re
import signal
import stat
import subprocess
import sys
import tarfile
import tempfile
from typing import Any, NoReturn


REPOSITORY = "CyberBASSLord-666/termux-mcp-edge"
ARCHIVE_NAME = "termux-qualified-runtime-image-v1.tar.gz"
LOCK_NAME = "termux-runtime-package-lock-v1.json"
SNAPSHOT_NAME = "termux-runtime-snapshot-v1.json"
OUTPUT_NAME = "termux-runtime-snapshot-replay-v1.json"
BASE_IMAGE = "termux/termux-docker:aarch64"
RUNTIME_USER = "1000:1000"
REQUESTED_PACKAGES = ["file", "jq", "python", "termux-services"]
CLAIM_BOUNDARY = {
    "physicalDeviceObserved": False,
    "androidFrameworkObserved": False,
    "sustainedPhysicalSoak": False,
    "physicalCertification": "not_run",
}
SHA_RE = re.compile(r"^[0-9a-f]{64}$")
OCI_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
RUN_RE = re.compile(r"^[1-9][0-9]*$")
PACKAGE_RE = re.compile(r"^[a-z0-9][a-z0-9+.-]{0,127}$")
ARCH_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,31}$")
VERSION_RE = re.compile(r"^[!-~]{1,128}$")
FILE_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9+._%~-]{0,255}$")
MAX_ARCHIVE_BYTES = 1_610_612_736
MAX_ARCHIVE_MEMBERS = 4096
MAX_ARCHIVE_EXPANDED_BYTES = 8_589_934_592


def fail(reason: str) -> NoReturn:
    print(f"[runtime-snapshot-replay] ERROR: {reason}", file=sys.stderr)
    raise SystemExit(1)


def closed_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("duplicate key")
        value[key] = item
    return value


def reject_constant(_value: str) -> NoReturn:
    raise ValueError("non-finite number")


def exact_keys(value: Any, expected: set[str], reason: str) -> None:
    if not isinstance(value, dict) or set(value) != expected:
        fail(reason)


def strict_json_equal(actual: Any, expected: Any) -> bool:
    if type(actual) is not type(expected):
        return False
    if isinstance(actual, dict):
        return (
            set(actual) == set(expected)
            and all(
                strict_json_equal(actual[key], expected[key])
                for key in actual
            )
        )
    if isinstance(actual, list):
        return (
            len(actual) == len(expected)
            and all(
                strict_json_equal(actual_item, expected_item)
                for actual_item, expected_item in zip(actual, expected)
            )
        )
    return bool(actual == expected)


def strict_int(value: Any, minimum: int, maximum: int, reason: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        fail(reason)
    if value < minimum or value > maximum:
        fail(reason)
    return value


def file_sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb", buffering=0) as source:
        while True:
            chunk = source.read(1024 * 1024)
            if not chunk:
                return digest.hexdigest()
            digest.update(chunk)


def read_regular(
    path_text: str,
    expected_name: str,
    maximum: int,
    reason: str,
) -> tuple[pathlib.Path, bytes]:
    path = pathlib.Path(path_text)
    if (
        not path.is_absolute()
        or pathlib.Path(os.path.normpath(path_text)) != path
        or path.name != expected_name
    ):
        fail(reason)
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.getuid()
            or metadata.st_size < 1
            or metadata.st_size > maximum
        ):
            fail(reason)
        chunks: list[bytes] = []
        remaining = metadata.st_size
        while remaining:
            chunk = os.read(descriptor, min(remaining, 1024 * 1024))
            if not chunk:
                fail(reason)
            chunks.append(chunk)
            remaining -= len(chunk)
        if os.read(descriptor, 1):
            fail(reason)
        return path, b"".join(chunks)
    except OSError:
        fail(reason)
    finally:
        if "descriptor" in locals():
            os.close(descriptor)


def inspect_large_regular(
    path_text: str,
    expected_name: str,
    maximum: int,
    reason: str,
) -> tuple[pathlib.Path, int, str]:
    path = pathlib.Path(path_text)
    if (
        not path.is_absolute()
        or pathlib.Path(os.path.normpath(path_text)) != path
        or path.name != expected_name
    ):
        fail(reason)
    descriptor = -1
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.getuid()
            or metadata.st_size < 1
            or metadata.st_size > maximum
        ):
            fail(reason)
        digest = hashlib.sha256()
        observed = 0
        while True:
            chunk = os.read(descriptor, 1024 * 1024)
            if not chunk:
                break
            observed += len(chunk)
            if observed > maximum:
                fail(reason)
            digest.update(chunk)
        if observed != metadata.st_size:
            fail(reason)
        return path, observed, digest.hexdigest()
    except OSError:
        fail(reason)
    finally:
        if descriptor >= 0:
            os.close(descriptor)


def parse_json(raw: bytes, reason: str) -> Any:
    try:
        text = raw.decode("utf-8")
        value = json.loads(
            text,
            object_pairs_hook=closed_object,
            parse_constant=reject_constant,
        )
    except (UnicodeError, ValueError, TypeError):
        fail(reason)
    return value


def validate_file_record(
    value: Any,
    expected_name: str,
    maximum: int,
    reason: str,
) -> None:
    exact_keys(value, {"fileName", "sha256", "bytes"}, reason)
    if (
        value["fileName"] != expected_name
        or not isinstance(value["sha256"], str)
        or SHA_RE.fullmatch(value["sha256"]) is None
    ):
        fail(reason)
    strict_int(value["bytes"], 1, maximum, reason)


def validate_claim_boundary(value: Any, reason: str) -> None:
    if value != CLAIM_BOUNDARY:
        fail(reason)


def validate_package_lock(
    value: Any,
    lock_path: pathlib.Path,
    lock_raw: bytes,
) -> None:
    exact_keys(
        value,
        {
            "schemaVersion",
            "lockVersion",
            "repository",
            "commit",
            "androidRunId",
            "base",
            "requestedPackages",
            "resolution",
            "installation",
            "repositoryIndexes",
            "packages",
        },
        "package_lock_contract_invalid",
    )
    if (
        value["schemaVersion"] != 1
        or value["lockVersion"] != "1"
        or value["repository"] != REPOSITORY
        or not isinstance(value["commit"], str)
        or COMMIT_RE.fullmatch(value["commit"]) is None
        or not isinstance(value["androidRunId"], str)
        or RUN_RE.fullmatch(value["androidRunId"]) is None
        or value["requestedPackages"] != REQUESTED_PACKAGES
        or value["resolution"]
        != {
            "resolver": "termux-apt-download-only",
            "repositoryMetadataAuthenticated": True,
            "packageBytesFrozenBeforeBuild": True,
            "finalImageBuildNetwork": "none",
        }
        or value["installation"]
        != {
            "method": "termux-dpkg-unpack-configure",
            "dependencyRepair": "none",
            "runtimeUser": "1000:1000",
        }
    ):
        fail("package_lock_contract_invalid")
    exact_keys(value["base"], {"image", "digest", "imageId"}, "package_lock_contract_invalid")
    if (
        value["base"]["image"] != BASE_IMAGE
        or not isinstance(value["base"]["digest"], str)
        or OCI_RE.fullmatch(value["base"]["digest"]) is None
        or not isinstance(value["base"]["imageId"], str)
        or OCI_RE.fullmatch(value["base"]["imageId"]) is None
    ):
        fail("package_lock_contract_invalid")
    indexes = value["repositoryIndexes"]
    if not isinstance(indexes, list) or not 1 <= len(indexes) <= 16:
        fail("package_lock_contract_invalid")
    index_names: set[str] = set()
    for record in indexes:
        validate_file_record(record, record.get("fileName") if isinstance(record, dict) else "", 16_777_216, "package_lock_contract_invalid")
        if FILE_RE.fullmatch(record["fileName"]) is None or record["fileName"] in index_names:
            fail("package_lock_contract_invalid")
        index_names.add(record["fileName"])
    packages = value["packages"]
    if not isinstance(packages, list) or not 1 <= len(packages) <= 512:
        fail("package_lock_contract_invalid")
    expected_order: list[tuple[str, str, str, str]] = []
    identities: set[tuple[str, str, str]] = set()
    names: set[str] = set()
    for record in packages:
        exact_keys(
            record,
            {"package", "version", "architecture", "fileName", "sha256", "bytes"},
            "package_lock_contract_invalid",
        )
        if (
            not isinstance(record["package"], str)
            or PACKAGE_RE.fullmatch(record["package"]) is None
            or not isinstance(record["version"], str)
            or VERSION_RE.fullmatch(record["version"]) is None
            or not isinstance(record["architecture"], str)
            or ARCH_RE.fullmatch(record["architecture"]) is None
            or not isinstance(record["fileName"], str)
            or re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9+._~-]{0,255}\.deb", record["fileName"]) is None
            or not isinstance(record["sha256"], str)
            or SHA_RE.fullmatch(record["sha256"]) is None
        ):
            fail("package_lock_contract_invalid")
        strict_int(record["bytes"], 1, 268_435_456, "package_lock_contract_invalid")
        identity = (record["package"], record["version"], record["architecture"])
        if identity in identities or record["fileName"] in names:
            fail("package_lock_contract_invalid")
        identities.add(identity)
        names.add(record["fileName"])
        expected_order.append((*identity, record["fileName"]))
    if expected_order != sorted(expected_order):
        fail("package_lock_contract_invalid")
    if file_sha256(lock_path) != hashlib.sha256(lock_raw).hexdigest():
        fail("package_lock_changed_during_validation")


def validate_snapshot(value: Any) -> None:
    exact_keys(
        value,
        {
            "schemaVersion",
            "snapshotVersion",
            "status",
            "failureCode",
            "releaseQualificationEligible",
            "repository",
            "commit",
            "androidRunId",
            "base",
            "runtimeImageId",
            "platform",
            "rootfsLayers",
            "packageLock",
            "installedPackages",
            "archive",
            "claimBoundary",
            "rebuildReproducibilityClaim",
        },
        "snapshot_contract_invalid",
    )
    if (
        value["schemaVersion"] != 1
        or value["snapshotVersion"] != "1"
        or value["status"] != "pass"
        or value["failureCode"] is not None
        or value["releaseQualificationEligible"] is not False
        or value["repository"] != REPOSITORY
        or not isinstance(value["commit"], str)
        or COMMIT_RE.fullmatch(value["commit"]) is None
        or not isinstance(value["androidRunId"], str)
        or RUN_RE.fullmatch(value["androidRunId"]) is None
        or not isinstance(value["runtimeImageId"], str)
        or OCI_RE.fullmatch(value["runtimeImageId"]) is None
        or value["platform"] != {"os": "linux", "architecture": "arm64"}
        or value["rebuildReproducibilityClaim"] is not False
    ):
        fail("snapshot_contract_invalid")
    exact_keys(value["base"], {"image", "digest", "imageId"}, "snapshot_contract_invalid")
    if (
        value["base"]["image"] != BASE_IMAGE
        or not isinstance(value["base"]["digest"], str)
        or OCI_RE.fullmatch(value["base"]["digest"]) is None
        or not isinstance(value["base"]["imageId"], str)
        or OCI_RE.fullmatch(value["base"]["imageId"]) is None
        or value["runtimeImageId"] == value["base"]["imageId"]
    ):
        fail("snapshot_contract_invalid")
    layers = value["rootfsLayers"]
    if (
        not isinstance(layers, list)
        or not 1 <= len(layers) <= 256
        or len(set(layers)) != len(layers)
        or any(not isinstance(item, str) or OCI_RE.fullmatch(item) is None for item in layers)
    ):
        fail("snapshot_contract_invalid")
    validate_file_record(value["packageLock"], LOCK_NAME, 16_777_216, "snapshot_contract_invalid")
    inventory = value["installedPackages"]
    exact_keys(inventory, {"sha256", "count", "packages"}, "snapshot_contract_invalid")
    if not isinstance(inventory["sha256"], str) or SHA_RE.fullmatch(inventory["sha256"]) is None:
        fail("snapshot_contract_invalid")
    packages = inventory["packages"]
    strict_int(inventory["count"], 1, 4096, "snapshot_contract_invalid")
    if not isinstance(packages, list) or len(packages) != inventory["count"]:
        fail("snapshot_contract_invalid")
    observed_order: list[tuple[str, str, str]] = []
    for package in packages:
        exact_keys(package, {"package", "version", "architecture"}, "snapshot_contract_invalid")
        if (
            not isinstance(package["package"], str)
            or PACKAGE_RE.fullmatch(package["package"]) is None
            or not isinstance(package["version"], str)
            or VERSION_RE.fullmatch(package["version"]) is None
            or not isinstance(package["architecture"], str)
            or ARCH_RE.fullmatch(package["architecture"]) is None
        ):
            fail("snapshot_contract_invalid")
        observed_order.append((package["package"], package["version"], package["architecture"]))
    if observed_order != sorted(set(observed_order)):
        fail("snapshot_contract_invalid")
    archive = value["archive"]
    exact_keys(
        archive,
        {"fileName", "format", "compression", "sha256", "bytes"},
        "snapshot_contract_invalid",
    )
    if (
        archive["fileName"] != ARCHIVE_NAME
        or archive["format"] != "docker-image-archive-v1"
        or archive["compression"] != "gzip-no-name"
        or not isinstance(archive["sha256"], str)
        or SHA_RE.fullmatch(archive["sha256"]) is None
    ):
        fail("snapshot_contract_invalid")
    strict_int(archive["bytes"], 1, MAX_ARCHIVE_BYTES, "snapshot_contract_invalid")
    validate_claim_boundary(value["claimBoundary"], "snapshot_contract_invalid")


def parse_archive(
    archive_path: pathlib.Path,
    expected_runtime_id: str,
    expected_commit: str,
    expected_layers: list[str],
) -> None:
    expected_digest = expected_runtime_id.removeprefix("sha256:")
    expected_config = f"{expected_digest}.json"
    expected_tag = f"termux-mcp-qualified-runtime:{expected_commit}"
    file_members: dict[str, tarfile.TarInfo] = {}
    directory_names: set[str] = set()
    expanded = 0
    layer_expanded = 0

    def read_member(
        archive: tarfile.TarFile,
        name: str,
        maximum: int,
        reason: str,
    ) -> bytes:
        member = file_members.get(name)
        if member is None or not 1 <= member.size <= maximum:
            fail(reason)
        source = archive.extractfile(member)
        if source is None:
            fail(reason)
        raw = source.read(member.size + 1)
        if len(raw) != member.size:
            fail(reason)
        return raw

    def member_sha256(
        archive: tarfile.TarFile,
        name: str,
        reason: str,
    ) -> str:
        member = file_members.get(name)
        if member is None:
            fail(reason)
        source = archive.extractfile(member)
        if source is None:
            fail(reason)
        digest = hashlib.sha256()
        observed = 0
        while True:
            chunk = source.read(1024 * 1024)
            if not chunk:
                break
            observed += len(chunk)
            if observed > member.size:
                fail(reason)
            digest.update(chunk)
        if observed != member.size:
            fail(reason)
        return digest.hexdigest()

    def layer_diff_id(
        archive: tarfile.TarFile,
        name: str,
        media_type: str,
        reason: str,
    ) -> str:
        nonlocal layer_expanded
        member = file_members.get(name)
        if member is None:
            fail(reason)
        source = archive.extractfile(member)
        if source is None:
            fail(reason)
        if media_type in {
            "application/vnd.oci.image.layer.v1.tar",
            "application/vnd.oci.image.layer.nondistributable.v1.tar",
        }:
            stream = source
        elif media_type in {
            "application/vnd.oci.image.layer.v1.tar+gzip",
            "application/vnd.oci.image.layer.nondistributable.v1.tar+gzip",
            "application/vnd.docker.image.rootfs.diff.tar.gzip",
            "application/vnd.docker.image.rootfs.foreign.diff.tar.gzip",
        }:
            try:
                stream = gzip.GzipFile(fileobj=source, mode="rb")
            except (OSError, gzip.BadGzipFile):
                fail(reason)
        else:
            fail("archive_layer_media_type_invalid")
        digest = hashlib.sha256()
        observed = 0
        try:
            while True:
                chunk = stream.read(1024 * 1024)
                if not chunk:
                    break
                chunk_size = len(chunk)
                observed += chunk_size
                layer_expanded += chunk_size
                if layer_expanded > MAX_ARCHIVE_EXPANDED_BYTES:
                    fail("archive_expanded_size_invalid")
                digest.update(chunk)
        except (OSError, EOFError, gzip.BadGzipFile):
            fail(reason)
        if observed < 1:
            fail(reason)
        return f"sha256:{digest.hexdigest()}"

    def validate_config(config_raw: bytes) -> dict[str, Any]:
        if hashlib.sha256(config_raw).hexdigest() != expected_digest:
            fail("archive_config_digest_mismatch")
        config = parse_json(config_raw, "archive_config_invalid")
        if (
            not isinstance(config, dict)
            or config.get("architecture") != "arm64"
            or config.get("os") != "linux"
            or not isinstance(config.get("config"), dict)
            or config["config"].get("User") != RUNTIME_USER
            or not isinstance(config.get("rootfs"), dict)
            or config["rootfs"].get("type") != "layers"
            or config["rootfs"].get("diff_ids") != expected_layers
        ):
            fail("archive_config_invalid")
        return config

    def validate_oci_legacy_metadata(
        archive: tarfile.TarFile,
        metadata_names: set[str],
        config: dict[str, Any],
        expected_diff_ids: list[str],
    ) -> None:
        reason = "archive_legacy_metadata_invalid"
        if len(metadata_names) != len(expected_diff_ids):
            fail(reason)
        runtime_config = config.get("config")
        created = config.get("created")
        if not isinstance(runtime_config, dict) or not isinstance(created, str):
            fail(reason)

        moby_v28_config_fields: list[tuple[str, Any, bool]] = [
            ("Hostname", "", False),
            ("Domainname", "", False),
            ("User", "", False),
            ("AttachStdin", False, False),
            ("AttachStdout", False, False),
            ("AttachStderr", False, False),
            ("ExposedPorts", None, True),
            ("Tty", False, False),
            ("OpenStdin", False, False),
            ("StdinOnce", False, False),
            ("Env", None, False),
            ("Cmd", None, False),
            ("Healthcheck", None, True),
            ("ArgsEscaped", False, True),
            ("Image", "", False),
            ("Volumes", None, False),
            ("WorkingDir", "", False),
            ("Entrypoint", None, False),
            ("NetworkDisabled", False, True),
            ("MacAddress", "", True),
            ("OnBuild", None, False),
            ("Labels", None, False),
            ("StopSignal", "", True),
            ("StopTimeout", None, True),
            ("Shell", None, True),
        ]
        moby_v29_config_fields = [
            (name, default, True if name == "OnBuild" else omit)
            for name, default, omit in moby_v28_config_fields
            if name != "MacAddress"
        ]

        def json_bytes(value: Any) -> bytes:
            try:
                text = json.dumps(
                    value,
                    ensure_ascii=False,
                    allow_nan=False,
                    separators=(",", ":"),
                )
            except (TypeError, ValueError):
                fail(reason)
            return (
                text.replace("&", r"\u0026")
                .replace("<", r"\u003c")
                .replace(">", r"\u003e")
                .replace("\u2028", r"\u2028")
                .replace("\u2029", r"\u2029")
                .encode()
            )

        def omitted(name: str, value: Any) -> bool:
            if name in {"ExposedPorts", "OnBuild", "Shell"}:
                return value is None or value == {} or value == []
            if name in {"Healthcheck", "StopTimeout"}:
                return value is None
            if name in {"ArgsEscaped", "NetworkDisabled"}:
                return value is False
            if name in {"MacAddress", "StopSignal"}:
                return value == ""
            return False

        def go_config_raw(
            sparse: dict[str, Any],
            fields: list[tuple[str, Any, bool]],
        ) -> bytes:
            allowed = {name for name, _default, _omit in fields}
            if set(sparse) - allowed:
                fail(reason)
            values = {
                name: default
                for name, default, _omit in fields
            }
            values.update(sparse)
            pairs = []
            for name, _default, omit_empty in fields:
                value = values[name]
                if omit_empty and omitted(name, value):
                    continue
                pairs.append(json_bytes(name) + b":" + json_bytes(value))
            return b"{" + b",".join(pairs) + b"}"

        def sorted_raw_map(fields: dict[str, bytes]) -> bytes:
            return b"{" + b",".join(
                json_bytes(key) + b":" + fields[key]
                for key in sorted(fields)
            ) + b"}"

        def chain_ids() -> list[str]:
            values: list[str] = []
            for diff_id in expected_diff_ids:
                if not values:
                    chain_id = diff_id
                else:
                    chain_id = "sha256:" + hashlib.sha256(
                        f"{values[-1]} {diff_id}".encode()
                    ).hexdigest()
                values.append(chain_id)
            return values

        def expected_metadata(
            config_fields: list[tuple[str, Any, bool]],
        ) -> dict[str, bytes]:
            values: dict[str, bytes] = {}
            parent_id: str | None = None
            chains = chain_ids()
            for index, chain_id in enumerate(chains):
                terminal = index == len(chains) - 1
                record_created = (
                    created if terminal else "1970-01-01T00:00:00Z"
                )
                create_fields = {
                    "container_config": go_config_raw({}, config_fields),
                    "created": json_bytes(record_created),
                    "layer_id": json_bytes(chain_id),
                }
                if parent_id is not None:
                    create_fields["parent"] = json_bytes(
                        f"sha256:{parent_id}"
                    )
                if terminal:
                    create_fields["config"] = go_config_raw(
                        runtime_config,
                        config_fields,
                    )
                    create_fields["architecture"] = json_bytes("arm64")
                    create_fields["os"] = json_bytes("linux")
                image_id = hashlib.sha256(
                    sorted_raw_map(create_fields)
                ).hexdigest()

                record_pairs: list[tuple[str, bytes]] = [
                    ("id", json_bytes(image_id)),
                ]
                if parent_id is not None:
                    record_pairs.append(("parent", json_bytes(parent_id)))
                record_pairs.extend(
                    [
                        ("created", json_bytes(record_created)),
                        (
                            "container_config",
                            go_config_raw({}, config_fields),
                        ),
                    ]
                )
                if terminal:
                    record_pairs.extend(
                        [
                            (
                                "config",
                                go_config_raw(runtime_config, config_fields),
                            ),
                            ("architecture", json_bytes("arm64")),
                        ]
                    )
                record_pairs.append(("os", json_bytes("linux")))
                raw = b"{" + b",".join(
                    json_bytes(key) + b":" + value
                    for key, value in record_pairs
                ) + b"}"
                name = f"blobs/sha256/{hashlib.sha256(raw).hexdigest()}"
                values[name] = raw
                parent_id = image_id
            return values

        observed: dict[str, bytes] = {}
        for name in metadata_names:
            digest = name.removeprefix("blobs/sha256/")
            if SHA_RE.fullmatch(digest) is None:
                fail(reason)
            raw = read_member(archive, name, 4_194_304, reason)
            if hashlib.sha256(raw).hexdigest() != digest:
                fail(reason)
            parse_json(raw, reason)
            observed[name] = raw

        accepted_profiles = (
            expected_metadata(moby_v28_config_fields),
            expected_metadata(moby_v29_config_fields),
        )
        if not any(observed == expected for expected in accepted_profiles):
            fail(reason)

    def validate_annotations(value: Any, reason: str) -> None:
        if not isinstance(value, dict) or len(value) > 64:
            fail(reason)
        for key, item in value.items():
            if (
                not isinstance(key, str)
                or not 1 <= len(key) <= 256
                or not isinstance(item, str)
                or not 1 <= len(item) <= 4096
            ):
                fail(reason)

    def validate_descriptor(
        value: Any,
        *,
        media_types: set[str],
        require_platform: bool,
        reason: str,
    ) -> None:
        if not isinstance(value, dict):
            fail(reason)
        required = {"mediaType", "digest", "size"}
        optional = {"annotations", "platform"}
        if not required.issubset(value) or not set(value).issubset(required | optional):
            fail(reason)
        if (
            value["mediaType"] not in media_types
            or not isinstance(value["digest"], str)
            or OCI_RE.fullmatch(value["digest"]) is None
        ):
            fail(reason)
        strict_int(value["size"], 1, MAX_ARCHIVE_EXPANDED_BYTES, reason)
        if "annotations" in value:
            validate_annotations(value["annotations"], reason)
        if require_platform and value.get("platform") != {
            "architecture": "arm64",
            "os": "linux",
        }:
            fail(reason)
        if not require_platform and "platform" in value and value["platform"] != {
            "architecture": "arm64",
            "os": "linux",
        }:
            fail(reason)

    def validate_repositories(
        raw: bytes,
        expected_layer_digest: str | None = None,
    ) -> None:
        value = parse_json(raw, "archive_repositories_invalid")
        if not isinstance(value, dict) or len(value) != 1:
            fail("archive_repositories_invalid")
        repository, tags = next(iter(value.items()))
        if (
            repository != "termux-mcp-qualified-runtime"
            or not isinstance(tags, dict)
            or set(tags) != {expected_commit}
            or not isinstance(tags[expected_commit], str)
            or re.fullmatch(r"[0-9a-f]{64}", tags[expected_commit]) is None
            or (
                expected_layer_digest is not None
                and tags[expected_commit] != expected_layer_digest
            )
        ):
            fail("archive_repositories_invalid")

    def validate_directories(allowed_files: set[str]) -> None:
        allowed_directories: set[str] = set()
        for file_name in allowed_files:
            parent = posixpath.dirname(file_name)
            while parent:
                allowed_directories.add(parent)
                parent = posixpath.dirname(parent)
        if not directory_names.issubset(allowed_directories):
            fail("archive_unreferenced_member")

    try:
        with tarfile.open(archive_path, mode="r:gz") as archive:
            members = archive.getmembers()
            if not 1 <= len(members) <= MAX_ARCHIVE_MEMBERS:
                fail("archive_structure_invalid")
            for member in members:
                name = member.name[:-1] if member.isdir() and member.name.endswith("/") else member.name
                if (
                    not name
                    or name == "."
                    or name.startswith("/")
                    or "\\" in name
                    or posixpath.normpath(name) != name
                    or name == ".."
                    or name.startswith("../")
                    or "/../" in name
                    or member.issym()
                    or member.islnk()
                    or member.ischr()
                    or member.isblk()
                    or member.isfifo()
                ):
                    fail("archive_structure_invalid")
                if member.isfile():
                    if name in file_members or name in directory_names:
                        fail("archive_structure_invalid")
                    file_members[name] = member
                    expanded += member.size
                    if expanded > MAX_ARCHIVE_EXPANDED_BYTES:
                        fail("archive_expanded_size_invalid")
                elif member.isdir():
                    if name in file_members or name in directory_names:
                        fail("archive_structure_invalid")
                    directory_names.add(name)
                else:
                    fail("archive_structure_invalid")
            if ("oci-layout" in file_members) != ("index.json" in file_members):
                fail("archive_structure_invalid")

            if "oci-layout" not in file_members:
                manifest_raw = read_member(
                    archive, "manifest.json", 4_194_304, "archive_manifest_invalid"
                )
                config_raw = read_member(
                    archive, expected_config, 16_777_216, "archive_config_invalid"
                )
                manifest = parse_json(manifest_raw, "archive_manifest_invalid")
                if not isinstance(manifest, list) or len(manifest) != 1:
                    fail("archive_manifest_invalid")
                entry = manifest[0]
                exact_keys(
                    entry,
                    {"Config", "RepoTags", "Layers"},
                    "archive_manifest_invalid",
                )
                layer_names = entry["Layers"]
                if (
                    entry["Config"] != expected_config
                    or entry["RepoTags"] != [expected_tag]
                    or not isinstance(layer_names, list)
                    or len(layer_names) != len(expected_layers)
                    or len(set(layer_names)) != len(layer_names)
                    or any(
                        not isinstance(layer, str) or layer not in file_members
                        for layer in layer_names
                    )
                ):
                    fail("archive_manifest_invalid")
                validate_config(config_raw)
                for layer_name, expected_diff_id in zip(layer_names, expected_layers):
                    if (
                        f"sha256:{member_sha256(archive, layer_name, 'archive_layer_invalid')}"
                        != expected_diff_id
                    ):
                        fail("archive_layer_diff_id_mismatch")
                allowed_files = {"manifest.json", expected_config, *layer_names}
                for layer_name in layer_names:
                    parent = posixpath.dirname(layer_name)
                    if not parent:
                        continue
                    version_name = f"{parent}/VERSION"
                    metadata_name = f"{parent}/json"
                    if version_name in file_members:
                        if read_member(
                            archive,
                            version_name,
                            64,
                            "archive_legacy_metadata_invalid",
                        ).strip() != b"1.0":
                            fail("archive_legacy_metadata_invalid")
                        allowed_files.add(version_name)
                    if metadata_name in file_members:
                        metadata = parse_json(
                            read_member(
                                archive,
                                metadata_name,
                                4_194_304,
                                "archive_legacy_metadata_invalid",
                            ),
                            "archive_legacy_metadata_invalid",
                        )
                        if not isinstance(metadata, dict):
                            fail("archive_legacy_metadata_invalid")
                        allowed_files.add(metadata_name)
                if "repositories" in file_members:
                    validate_repositories(
                        read_member(
                            archive,
                            "repositories",
                            4_194_304,
                            "archive_repositories_invalid",
                        )
                    )
                    allowed_files.add("repositories")
                if set(file_members) != allowed_files:
                    fail("archive_unreferenced_member")
                validate_directories(allowed_files)
                return

            layout = parse_json(
                read_member(archive, "oci-layout", 4096, "archive_oci_layout_invalid"),
                "archive_oci_layout_invalid",
            )
            if layout != {"imageLayoutVersion": "1.0.0"}:
                fail("archive_oci_layout_invalid")
            index = parse_json(
                read_member(archive, "index.json", 4_194_304, "archive_oci_index_invalid"),
                "archive_oci_index_invalid",
            )
            exact_keys(
                index,
                {"schemaVersion", "mediaType", "manifests"},
                "archive_oci_index_invalid",
            )
            if (
                index["schemaVersion"] != 2
                or index["mediaType"] != "application/vnd.oci.image.index.v1+json"
                or not isinstance(index["manifests"], list)
                or len(index["manifests"]) != 1
            ):
                fail("archive_oci_index_invalid")
            image_descriptor = index["manifests"][0]
            validate_descriptor(
                image_descriptor,
                media_types={
                    "application/vnd.oci.image.manifest.v1+json",
                    "application/vnd.docker.distribution.manifest.v2+json",
                },
                require_platform=False,
                reason="archive_oci_index_invalid",
            )
            tag_bound = False
            annotations = image_descriptor.get("annotations", {})
            if annotations:
                image_name = annotations.get("io.containerd.image.name")
                reference = annotations.get("org.opencontainers.image.ref.name")
                tag_bound = (
                    isinstance(image_name, str)
                    and (
                        image_name == expected_tag
                        or image_name.endswith(f"/{expected_tag}")
                    )
                    and reference in (expected_commit, expected_tag)
                )
            manifest_digest = image_descriptor["digest"].removeprefix("sha256:")
            manifest_name = f"blobs/sha256/{manifest_digest}"
            manifest_raw = read_member(
                archive,
                manifest_name,
                16_777_216,
                "archive_oci_manifest_invalid",
            )
            if (
                len(manifest_raw) != image_descriptor["size"]
                or hashlib.sha256(manifest_raw).hexdigest() != manifest_digest
            ):
                fail("archive_oci_manifest_descriptor_mismatch")
            manifest = parse_json(manifest_raw, "archive_oci_manifest_invalid")
            if (
                not isinstance(manifest, dict)
                or not {"schemaVersion", "mediaType", "config", "layers"}.issubset(manifest)
                or not set(manifest).issubset(
                    {"schemaVersion", "mediaType", "config", "layers", "annotations"}
                )
                or manifest["schemaVersion"] != 2
                or manifest["mediaType"] != image_descriptor["mediaType"]
                or not isinstance(manifest["layers"], list)
                or len(manifest["layers"]) != len(expected_layers)
            ):
                fail("archive_oci_manifest_invalid")
            if "annotations" in manifest:
                validate_annotations(
                    manifest["annotations"],
                    "archive_oci_manifest_invalid",
                )
            config_descriptor = manifest["config"]
            validate_descriptor(
                config_descriptor,
                media_types={
                    "application/vnd.oci.image.config.v1+json",
                    "application/vnd.docker.container.image.v1+json",
                },
                require_platform=False,
                reason="archive_oci_config_descriptor_invalid",
            )
            if config_descriptor["digest"] != expected_runtime_id:
                fail("archive_config_digest_mismatch")
            config_name = f"blobs/sha256/{expected_digest}"
            config_raw = read_member(
                archive,
                config_name,
                16_777_216,
                "archive_config_invalid",
            )
            if len(config_raw) != config_descriptor["size"]:
                fail("archive_oci_config_descriptor_mismatch")
            validated_config = validate_config(config_raw)

            referenced_blobs = {manifest_name, config_name}
            layer_names = []
            layer_descriptors = manifest["layers"]
            for descriptor, expected_diff_id in zip(
                layer_descriptors,
                expected_layers,
            ):
                validate_descriptor(
                    descriptor,
                    media_types={
                        "application/vnd.oci.image.layer.v1.tar",
                        "application/vnd.oci.image.layer.v1.tar+gzip",
                        "application/vnd.oci.image.layer.nondistributable.v1.tar",
                        "application/vnd.oci.image.layer.nondistributable.v1.tar+gzip",
                        "application/vnd.docker.image.rootfs.diff.tar.gzip",
                        "application/vnd.docker.image.rootfs.foreign.diff.tar.gzip",
                    },
                    require_platform=False,
                    reason="archive_oci_layer_descriptor_invalid",
                )
                layer_digest = descriptor["digest"].removeprefix("sha256:")
                layer_name = f"blobs/sha256/{layer_digest}"
                member = file_members.get(layer_name)
                if (
                    member is None
                    or member.size != descriptor["size"]
                    or member_sha256(archive, layer_name, "archive_layer_invalid")
                    != layer_digest
                    or layer_diff_id(
                        archive,
                        layer_name,
                        descriptor["mediaType"],
                        "archive_layer_invalid",
                    )
                    != expected_diff_id
                ):
                    fail("archive_layer_diff_id_mismatch")
                if layer_name in referenced_blobs:
                    fail("archive_oci_layer_descriptor_invalid")
                referenced_blobs.add(layer_name)
                layer_names.append(layer_name)

            allowed_files = {
                "oci-layout",
                "index.json",
                *referenced_blobs,
            }
            has_layer_sources = False
            if "manifest.json" in file_members:
                legacy = parse_json(
                    read_member(
                        archive,
                        "manifest.json",
                        4_194_304,
                        "archive_manifest_invalid",
                    ),
                    "archive_manifest_invalid",
                )
                if not isinstance(legacy, list) or len(legacy) != 1:
                    fail("archive_manifest_invalid")
                entry = legacy[0]
                if not isinstance(entry, dict) or set(entry) not in (
                    {"Config", "RepoTags", "Layers"},
                    {"Config", "RepoTags", "Layers", "LayerSources"},
                ):
                    fail("archive_manifest_invalid")
                if (
                    entry["Config"] not in (expected_config, config_name)
                    or entry["RepoTags"] != [expected_tag]
                    or entry["Layers"] != layer_names
                ):
                    fail("archive_manifest_invalid")
                has_layer_sources = "LayerSources" in entry
                if has_layer_sources:
                    if any(
                        descriptor["mediaType"]
                        != "application/vnd.oci.image.layer.v1.tar"
                        or descriptor["digest"] != expected_diff_id
                        for descriptor, expected_diff_id in zip(
                            layer_descriptors,
                            expected_layers,
                        )
                    ):
                        fail("archive_manifest_invalid")
                    layer_sources = entry["LayerSources"]
                    expected_sources = {
                        expected_diff_id: {
                            "mediaType": descriptor["mediaType"],
                            "size": descriptor["size"],
                            "digest": descriptor["digest"],
                        }
                        for descriptor, expected_diff_id in zip(
                            layer_descriptors,
                            expected_layers,
                        )
                    }
                    if (
                        not isinstance(layer_sources, dict)
                        or len(layer_sources) != len(layer_descriptors)
                        or not strict_json_equal(
                            layer_sources,
                            expected_sources,
                        )
                    ):
                        fail("archive_manifest_invalid")
                tag_bound = True
                allowed_files.add("manifest.json")
            if "repositories" in file_members:
                validate_repositories(
                    read_member(
                        archive,
                        "repositories",
                        4_194_304,
                        "archive_repositories_invalid",
                    ),
                    (
                        expected_layers[-1]
                        if has_layer_sources
                        else layer_descriptors[-1]["digest"]
                    ).removeprefix("sha256:"),
                )
                tag_bound = True
                allowed_files.add("repositories")
            if not tag_bound:
                fail("archive_runtime_tag_unbound")
            metadata_names = set(file_members) - allowed_files
            if "manifest.json" in file_members and has_layer_sources:
                if "repositories" not in file_members:
                    fail("archive_repositories_invalid")
                if any(
                    re.fullmatch(r"blobs/sha256/[0-9a-f]{64}", name) is None
                    for name in metadata_names
                ):
                    fail("archive_unreferenced_member")
                validate_oci_legacy_metadata(
                    archive,
                    metadata_names,
                    validated_config,
                    expected_layers,
                )
                allowed_files.update(metadata_names)
            if set(file_members) != allowed_files:
                fail("archive_unreferenced_member")
            validate_directories(allowed_files)
    except (OSError, tarfile.TarError, gzip.BadGzipFile):
        fail("archive_structure_invalid")


def run_checked(arguments: list[str], timeout: int, reason: str) -> subprocess.CompletedProcess[str]:
    try:
        result = subprocess.run(
            arguments,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        fail(reason)
    if result.returncode != 0:
        fail(reason)
    return result


def docker_image_exists(reference: str) -> bool:
    try:
        result = subprocess.run(
            ["docker", "image", "inspect", reference],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=20,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        fail("docker_inspect_failed")
    return result.returncode == 0


def parse_runtime_probe(
    raw: str,
) -> tuple[
    list[dict[str, str]],
    list[dict[str, Any]],
    list[dict[str, Any]],
    dict[str, Any],
    str,
    int,
]:
    lines = raw.splitlines()
    markers = (
        "__TERMUX_MCP_PACKAGES__",
        "__TERMUX_MCP_PACKAGE_INPUTS__",
        "__TERMUX_MCP_REPOSITORY_INDEXES__",
        "__TERMUX_MCP_PACKAGE_LOCK__",
        "__TERMUX_MCP_LINKER__",
    )
    if any(lines.count(marker) != 1 for marker in markers):
        fail("runtime_probe_invalid")
    (
        package_index,
        package_input_index,
        index_input_index,
        package_lock_index,
        linker_index,
    ) = (
        lines.index(marker) for marker in markers
    )
    if (
        package_index != 0
        or package_input_index <= package_index + 1
        or index_input_index <= package_input_index + 1
        or package_lock_index <= index_input_index + 1
        or package_lock_index + 2 != linker_index
        or linker_index + 1 != len(lines) - 1
    ):
        fail("runtime_probe_invalid")
    packages: list[dict[str, str]] = []
    identities: set[tuple[str, str, str]] = set()
    for line in lines[package_index + 1 : package_input_index]:
        parts = line.split("\t")
        if len(parts) != 3:
            fail("runtime_probe_invalid")
        package, version, architecture = parts
        identity = (package, version, architecture)
        if (
            PACKAGE_RE.fullmatch(package) is None
            or VERSION_RE.fullmatch(version) is None
            or ARCH_RE.fullmatch(architecture) is None
            or identity in identities
        ):
            fail("runtime_probe_invalid")
        identities.add(identity)
        packages.append(
            {"package": package, "version": version, "architecture": architecture}
        )
    if not packages or [
        (item["package"], item["version"], item["architecture"]) for item in packages
    ] != sorted(identities):
        fail("runtime_probe_invalid")

    def provenance_records(
        records: list[str],
        *,
        packages_only: bool,
    ) -> list[dict[str, Any]]:
        values: list[dict[str, Any]] = []
        names: set[str] = set()
        for line in records:
            parts = line.split("\t")
            if len(parts) != 3:
                fail("runtime_probe_invalid")
            file_name, sha256, raw_bytes = parts
            pattern = (
                r"[A-Za-z0-9][A-Za-z0-9+._~-]{0,255}\.deb"
                if packages_only
                else r"[A-Za-z0-9][A-Za-z0-9+._%~-]{0,255}"
            )
            if (
                re.fullmatch(pattern, file_name) is None
                or file_name in names
                or SHA_RE.fullmatch(sha256) is None
                or not raw_bytes.isdigit()
            ):
                fail("runtime_probe_invalid")
            byte_count = int(raw_bytes)
            maximum = 268_435_456 if packages_only else 16_777_216
            if not 1 <= byte_count <= maximum:
                fail("runtime_probe_invalid")
            names.add(file_name)
            values.append(
                {"fileName": file_name, "sha256": sha256, "bytes": byte_count}
            )
        if not values or [item["fileName"] for item in values] != sorted(names):
            fail("runtime_probe_invalid")
        return values

    package_inputs = provenance_records(
        lines[package_input_index + 1 : index_input_index],
        packages_only=True,
    )
    repository_indexes = provenance_records(
        lines[index_input_index + 1 : package_lock_index],
        packages_only=False,
    )
    package_lock_records = provenance_records(
        lines[package_lock_index + 1 : linker_index],
        packages_only=False,
    )
    if len(package_lock_records) != 1 or package_lock_records[0]["fileName"] != LOCK_NAME:
        fail("runtime_probe_invalid")
    runtime_package_lock = package_lock_records[0]
    linker = lines[linker_index + 1].split("\t")
    if (
        len(linker) != 2
        or SHA_RE.fullmatch(linker[0]) is None
        or not linker[1].isdigit()
    ):
        fail("runtime_probe_invalid")
    linker_bytes = int(linker[1])
    if not 1 <= linker_bytes <= 16_777_216:
        fail("runtime_probe_invalid")
    return (
        packages,
        package_inputs,
        repository_indexes,
        runtime_package_lock,
        linker[0],
        linker_bytes,
    )


def main(argv: list[str]) -> None:
    commit_helper = pathlib.Path(argv[0])
    parser = argparse.ArgumentParser(add_help=False, allow_abbrev=False)
    parser.add_argument("--archive", required=True)
    parser.add_argument("--package-lock", required=True)
    parser.add_argument("--snapshot-manifest", required=True)
    parser.add_argument("--aggregate-evidence", required=True)
    parser.add_argument("--deployment-evidence", required=True)
    parser.add_argument("--output", required=True)
    try:
        args = parser.parse_args(argv[1:])
    except SystemExit:
        fail("arguments_invalid")

    if os.uname().machine not in ("aarch64", "arm64"):
        fail("native_arm64_runner_required")
    archive_path, archive_bytes, archive_sha = inspect_large_regular(
        args.archive, ARCHIVE_NAME, MAX_ARCHIVE_BYTES, "archive_file_invalid"
    )
    lock_path, lock_raw = read_regular(
        args.package_lock, LOCK_NAME, 16_777_216, "package_lock_file_invalid"
    )
    snapshot_path, snapshot_raw = read_regular(
        args.snapshot_manifest, SNAPSHOT_NAME, 16_777_216, "snapshot_file_invalid"
    )
    _aggregate_path, aggregate_raw = read_regular(
        args.aggregate_evidence,
        "termux-native-aggregate-evidence-v4.json",
        16_777_216,
        "aggregate_file_invalid",
    )
    _deployment_path, deployment_raw = read_regular(
        args.deployment_evidence,
        "automated-native-deployment-v1.json",
        16_777_216,
        "deployment_file_invalid",
    )
    package_lock = parse_json(lock_raw, "package_lock_json_invalid")
    snapshot = parse_json(snapshot_raw, "snapshot_json_invalid")
    aggregate = parse_json(aggregate_raw, "aggregate_json_invalid")
    deployment = parse_json(deployment_raw, "deployment_json_invalid")
    validate_package_lock(package_lock, lock_path, lock_raw)
    validate_snapshot(snapshot)

    if (
        snapshot["commit"] != package_lock["commit"]
        or snapshot["androidRunId"] != package_lock["androidRunId"]
        or snapshot["base"] != package_lock["base"]
        or snapshot["packageLock"]
        != {
            "fileName": LOCK_NAME,
            "sha256": hashlib.sha256(lock_raw).hexdigest(),
            "bytes": len(lock_raw),
        }
        or snapshot["archive"]["sha256"] != archive_sha
        or snapshot["archive"]["bytes"] != archive_bytes
    ):
        fail("snapshot_input_join_mismatch")
    try:
        aggregate_candidate = aggregate["candidate"]
        aggregate_environment = aggregate["environment"]
        deployment_candidate = deployment["candidate"]
        deployment_environment = deployment["environment"]
        linker = deployment_environment["androidLinker"]
    except (KeyError, TypeError):
        fail("component_environment_invalid")
    if (
        aggregate_candidate.get("commit") != snapshot["commit"]
        or aggregate_candidate.get("androidRunId") != snapshot["androidRunId"]
        or deployment_candidate.get("commit") != snapshot["commit"]
        or deployment_candidate.get("nativeRunId") != snapshot["androidRunId"]
        or aggregate_environment.get("image") != BASE_IMAGE
        or aggregate_environment.get("imageDigest") != snapshot["base"]["digest"]
        or aggregate_environment.get("rootfsImageId") != snapshot["base"]["imageId"]
        or aggregate_environment.get("runtimeImageDigest") != snapshot["runtimeImageId"]
        or deployment_environment.get("rootfsImage") != BASE_IMAGE
        or deployment_environment.get("rootfsDigest") != snapshot["base"]["digest"]
        or deployment_environment.get("rootfsImageId") != snapshot["base"]["imageId"]
        or deployment_environment.get("runtimeImageDigest") != snapshot["runtimeImageId"]
        or not isinstance(linker, dict)
        or linker.get("observed") is not True
        or linker.get("path") != "/system/bin/linker64"
        or not isinstance(linker.get("sha256"), str)
        or SHA_RE.fullmatch(linker["sha256"]) is None
    ):
        fail("component_environment_mismatch")
    strict_int(linker.get("bytes"), 1, 16_777_216, "component_environment_mismatch")

    runtime_id = snapshot["runtimeImageId"]
    expected_tag = f"termux-mcp-qualified-runtime:{snapshot['commit']}"
    parse_archive(
        archive_path,
        runtime_id,
        snapshot["commit"],
        snapshot["rootfsLayers"],
    )
    if docker_image_exists(runtime_id) or docker_image_exists(expected_tag):
        fail("runtime_image_preexisting")

    loaded = False
    try:
        run_checked(
            ["docker", "load", "--input", str(archive_path)],
            600,
            "runtime_archive_load_failed",
        )
        loaded = True
        if not docker_image_exists(runtime_id) or not docker_image_exists(expected_tag):
            fail("runtime_image_identity_missing")
        inspect = run_checked(
            ["docker", "image", "inspect", runtime_id],
            60,
            "runtime_image_inspect_failed",
        )
        inspected = parse_json(inspect.stdout.encode(), "runtime_image_inspect_invalid")
        if not isinstance(inspected, list) or len(inspected) != 1:
            fail("runtime_image_inspect_invalid")
        image = inspected[0]
        if (
            image.get("Id") != runtime_id
            or image.get("Os") != "linux"
            or image.get("Architecture") != "arm64"
            or not isinstance(image.get("Config"), dict)
            or image["Config"].get("User") != RUNTIME_USER
            or not isinstance(image.get("RootFS"), dict)
            or image["RootFS"].get("Type") != "layers"
            or image["RootFS"].get("Layers") != snapshot["rootfsLayers"]
            or expected_tag not in (image.get("RepoTags") or [])
        ):
            fail("runtime_image_inspect_invalid")

        probe_script = r'''
set -euo pipefail
test "$(id -u):$(id -g)" = "1000:1000"
for required in file jq python python3 sv runsv runsvdir; do
  command -v "$required" >/dev/null
done
test -f /system/bin/linker64
test ! -L /system/bin/linker64
package_input_root=/data/data/com.termux/files/usr/share/termux-mcp/runtime-packages
repository_index_root=/data/data/com.termux/files/usr/share/termux-mcp/runtime-repository-indexes
package_lock_path=/data/data/com.termux/files/usr/share/termux-mcp/termux-runtime-package-lock-v1.json
for provenance_root in "$package_input_root" "$repository_index_root"; do
  test -d "$provenance_root"
  test ! -L "$provenance_root"
  test "$(find "$provenance_root" -mindepth 1 -maxdepth 1 ! -type f | wc -l)" = 0
  test "$(find "$provenance_root" -mindepth 2 | wc -l)" = 0
done
printf '__TERMUX_MCP_PACKAGES__\n'
dpkg-query -W -f='${Package}\t${Version}\t${Architecture}\n' | LC_ALL=C sort
printf '__TERMUX_MCP_PACKAGE_INPUTS__\n'
while IFS= read -r -d '' input; do
  printf '%s\t%s\t%s\n' \
    "$(basename "$input")" \
    "$(sha256sum "$input" | awk '{print $1}')" \
    "$(stat -c %s "$input")"
done < <(find "$package_input_root" -mindepth 1 -maxdepth 1 -type f -print0 | LC_ALL=C sort -z)
printf '__TERMUX_MCP_REPOSITORY_INDEXES__\n'
while IFS= read -r -d '' input; do
  printf '%s\t%s\t%s\n' \
    "$(basename "$input")" \
    "$(sha256sum "$input" | awk '{print $1}')" \
    "$(stat -c %s "$input")"
done < <(find "$repository_index_root" -mindepth 1 -maxdepth 1 -type f -print0 | LC_ALL=C sort -z)
printf '__TERMUX_MCP_PACKAGE_LOCK__\n'
test -f "$package_lock_path"
test ! -L "$package_lock_path"
printf '%s\t%s\t%s\n' \
  "$(basename "$package_lock_path")" \
  "$(sha256sum "$package_lock_path" | awk '{print $1}')" \
  "$(stat -c %s "$package_lock_path")"
printf '__TERMUX_MCP_LINKER__\n'
printf '%s\t%s\n' \
  "$(sha256sum /system/bin/linker64 | awk '{print $1}')" \
  "$(stat -c %s /system/bin/linker64)"
'''
        probe = run_checked(
            [
                "docker",
                "run",
                "--rm",
                "--network",
                "none",
                "--security-opt",
                "seccomp=unconfined",
                runtime_id,
                "bash",
                "-c",
                probe_script,
            ],
            300,
            "runtime_probe_failed",
        )
        (
            packages,
            package_inputs,
            repository_indexes,
            runtime_package_lock,
            linker_sha,
            linker_bytes,
        ) = parse_runtime_probe(probe.stdout)
        inventory_bytes = "".join(
            f"{item['package']}\t{item['version']}\t{item['architecture']}\n"
            for item in packages
        ).encode()
        inventory_sha = hashlib.sha256(inventory_bytes).hexdigest()
        expected_package_inputs = sorted(
            (
                {
                    "fileName": item["fileName"],
                    "sha256": item["sha256"],
                    "bytes": item["bytes"],
                }
                for item in package_lock["packages"]
            ),
            key=lambda item: item["fileName"],
        )
        expected_repository_indexes = sorted(
            package_lock["repositoryIndexes"],
            key=lambda item: item["fileName"],
        )
        expected_package_lock = {
            "fileName": LOCK_NAME,
            "sha256": hashlib.sha256(lock_raw).hexdigest(),
            "bytes": len(lock_raw),
        }
        locked_identities = {
            (item["package"], item["version"], item["architecture"])
            for item in package_lock["packages"]
        }
        installed_identities = {
            (item["package"], item["version"], item["architecture"])
            for item in packages
        }
        if not locked_identities.issubset(installed_identities):
            fail("runtime_package_installation_mismatch")
        if (
            packages != snapshot["installedPackages"]["packages"]
            or len(packages) != snapshot["installedPackages"]["count"]
            or inventory_sha != snapshot["installedPackages"]["sha256"]
            or package_inputs != expected_package_inputs
            or repository_indexes != expected_repository_indexes
            or runtime_package_lock != expected_package_lock
            or linker_sha != linker["sha256"]
            or linker_bytes != linker["bytes"]
        ):
            fail("runtime_replay_mismatch")
    finally:
        if loaded:
            subprocess.run(
                ["docker", "image", "rm", "--force", expected_tag],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=60,
                check=False,
            )
            subprocess.run(
                ["docker", "image", "rm", "--force", runtime_id],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=60,
                check=False,
            )

    snapshot_record = {
        "fileName": SNAPSHOT_NAME,
        "sha256": hashlib.sha256(snapshot_raw).hexdigest(),
        "bytes": len(snapshot_raw),
    }
    lock_record = {
        "fileName": LOCK_NAME,
        "sha256": hashlib.sha256(lock_raw).hexdigest(),
        "bytes": len(lock_raw),
    }
    report = {
        "schemaVersion": 1,
        "replayVersion": "1",
        "status": "pass",
        "failureCode": None,
        "releaseQualificationEligible": False,
        "repository": REPOSITORY,
        "commit": snapshot["commit"],
        "runtimeImageId": runtime_id,
        "snapshot": {
            "manifest": snapshot_record,
            "archive": snapshot["archive"],
        },
        "packageLock": lock_record,
        "installedPackages": {
            "sha256": snapshot["installedPackages"]["sha256"],
            "count": snapshot["installedPackages"]["count"],
        },
        "androidLinker": {
            "path": "/system/bin/linker64",
            "sha256": linker_sha,
            "bytes": linker_bytes,
        },
        "verification": {
            "archiveDigestVerified": True,
            "singleImageArchive": True,
            "loadedImageIdVerified": True,
            "platformVerified": True,
            "runtimeUserVerified": True,
            "rootfsLayersVerified": True,
            "packageLockVerified": True,
            "packageInputBytesVerified": True,
            "repositoryIndexBytesVerified": True,
            "installedPackageInventoryVerified": True,
            "requiredRuntimeCommandsVerified": True,
            "androidLinkerVerified": True,
            "runtimeNetworkAccess": False,
        },
        "claimBoundary": CLAIM_BOUNDARY,
        "rebuildReproducibilityClaim": False,
    }

    output = pathlib.Path(args.output)
    if (
        not output.is_absolute()
        or pathlib.Path(os.path.normpath(args.output)) != output
        or output.name != OUTPUT_NAME
        or output.exists()
        or output.is_symlink()
    ):
        fail("output_path_invalid")
    try:
        parent = output.parent
        parent_stat = parent.stat(follow_symlinks=False)
        if (
            not stat.S_ISDIR(parent_stat.st_mode)
            or stat.S_ISLNK(parent_stat.st_mode)
            or stat.S_IMODE(parent_stat.st_mode) != 0o700
            or parent_stat.st_uid != os.getuid()
            or parent.resolve(strict=True) != parent
        ):
            fail("output_parent_invalid")
    except (OSError, RuntimeError):
        fail("output_parent_invalid")

    encoded = (
        json.dumps(report, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
        + "\n"
    ).encode()
    if not 1 <= len(encoded) <= 262_144:
        fail("output_size_invalid")
    temporary: pathlib.Path | None = None
    try:
        descriptor, temporary_name = tempfile.mkstemp(
            prefix=".runtime-snapshot-replay.",
            dir=parent,
        )
        temporary = pathlib.Path(temporary_name)
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "wb") as destination:
            destination.write(encoded)
            destination.flush()
            os.fsync(destination.fileno())
        digest = hashlib.sha256(encoded).hexdigest()
        run_checked(
            [
                sys.executable,
                str(commit_helper),
                "--source",
                str(temporary),
                "--destination",
                str(output),
                "--sha256",
                digest,
                "--mode",
                "600",
            ],
            60,
            "output_publication_failed",
        )
        published_stat = output.stat(follow_symlinks=False)
        if (
            not stat.S_ISREG(published_stat.st_mode)
            or stat.S_IMODE(published_stat.st_mode) != 0o600
            or published_stat.st_uid != os.getuid()
            or published_stat.st_ino != temporary.stat(follow_symlinks=False).st_ino
            or file_sha256(output) != digest
        ):
            fail("output_identity_invalid")
    except OSError:
        fail("output_publication_failed")
    finally:
        if temporary is not None:
            try:
                temporary.unlink()
            except FileNotFoundError:
                pass
            except OSError:
                fail("private_cleanup_failed")

    print(
        "[runtime-snapshot-replay] status=pass "
        f"runtimeImageId={runtime_id} archiveSha256={archive_sha}"
    )


if __name__ == "__main__":
    main(sys.argv[1:])
PY

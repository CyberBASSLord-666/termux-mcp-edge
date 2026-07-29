#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
VERIFIER="$ROOT/scripts/verify_runtime_snapshot.sh"
LOCK_SCHEMA="$ROOT/docs/runtime-package-lock-schema-v1.json"
SNAPSHOT_SCHEMA="$ROOT/docs/runtime-snapshot-schema-v1.json"
REPLAY_SCHEMA="$ROOT/docs/runtime-snapshot-replay-schema-v1.json"
TMP="$(mktemp -d)"
trap 'rm -rf -- "$TMP"' EXIT

fail() {
  printf 'runtime snapshot replay test failed: %s\n' "$1" >&2
  exit 1
}

[[ -x "$VERIFIER" && -f "$VERIFIER" && ! -L "$VERIFIER" ]] \
  || fail verifier_missing_or_not_executable
for schema in "$LOCK_SCHEMA" "$SNAPSHOT_SCHEMA" "$REPLAY_SCHEMA"; do
  jq -e '
    ."$schema" == "https://json-schema.org/draft/2020-12/schema"
    and .type == "object"
    and .additionalProperties == false
  ' "$schema" >/dev/null || fail "closed schema invalid: $schema"
done

mkdir -m 700 "$TMP/bin" "$TMP/site" "$TMP/fixture"
cat >"$TMP/site/sitecustomize.py" <<'PY'
import os

if os.environ.get("TERMUX_MCP_TEST_FAKE_ARM64") == "1":
    class FakeUname:
        machine = "aarch64"
    os.uname = lambda: FakeUname()
PY

cat >"$TMP/bin/docker" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
state="${MOCK_DOCKER_STATE:?}"
case "${1:-} ${2:-}" in
  "image inspect")
    [[ -f "$state" ]] || exit 1
    reference="${3:-}"
    [[ "$reference" == "$MOCK_RUNTIME_ID" || "$reference" == "$MOCK_RUNTIME_TAG" ]] \
      || exit 1
    printf '[{"Id":"%s","Os":"linux","Architecture":"arm64","Config":{"User":"%s"},"RepoTags":["%s"],"RootFS":{"Type":"layers","Layers":%s}}]\n' \
      "$MOCK_RUNTIME_ID" "$MOCK_RUNTIME_USER" "$MOCK_RUNTIME_TAG" \
      "$MOCK_RUNTIME_LAYERS_JSON"
    ;;
  "load --input")
    [[ -f "${3:-}" ]]
    : >"$state"
    printf 'Loaded image: %s\n' "$MOCK_RUNTIME_TAG"
    ;;
  "run --rm")
    [[ -f "$state" ]]
    [[ "${MOCK_RUNTIME_UID_GID:?}" == "1000:1000" ]]
    [[ "$*" == *'test "$(id -u):$(id -g)" = "1000:1000"'* ]]
    [[ "$*" == *'package_lock_path=/data/data/com.termux/files/usr/share/termux-mcp/termux-runtime-package-lock-v1.json'* ]]
    printf '__TERMUX_MCP_PACKAGES__\n'
    printf '%b' "$MOCK_RUNTIME_PACKAGES"
    printf '__TERMUX_MCP_PACKAGE_INPUTS__\n'
    printf '%b' "$MOCK_RUNTIME_PACKAGE_INPUTS"
    printf '__TERMUX_MCP_REPOSITORY_INDEXES__\n'
    printf '%b' "$MOCK_RUNTIME_REPOSITORY_INDEXES"
    printf '__TERMUX_MCP_PACKAGE_LOCK__\n'
    printf '%b' "$MOCK_RUNTIME_PACKAGE_LOCK"
    printf '__TERMUX_MCP_LINKER__\n'
    printf '%s\t%s\n' "$MOCK_LINKER_SHA" "$MOCK_LINKER_BYTES"
    ;;
  "image rm")
    rm -f -- "$state"
    ;;
  *)
    printf 'unexpected mock docker invocation: %q ' "$@" >&2
    printf '\n' >&2
    exit 90
    ;;
esac
SH
chmod 755 "$TMP/bin/docker"

python3 - "$TMP/fixture" <<'PY'
import gzip
import hashlib
import io
import json
import pathlib
import tarfile
import sys

root = pathlib.Path(sys.argv[1])
commit = "a" * 40
run_id = "12345"
base_digest = "sha256:" + "b" * 64
base_id = "sha256:" + "c" * 64
layer_raws = [f"fixture-layer-{index}".encode() for index in range(1, 7)]
layers = [
    "sha256:" + hashlib.sha256(layer_raw).hexdigest()
    for layer_raw in layer_raws
]
config = json.dumps(
    {
        "architecture": "arm64",
        "config": {
            "User": "1000:1000",
            "Env": [
                "PATH=/data/data/com.termux/files/usr/bin",
                "ANDROID_DATA=/data",
                "ANDROID_ROOT=/system",
                "HOME=/data/data/com.termux/files/home",
                "LANG=en_US.UTF-8",
                "PREFIX=/data/data/com.termux/files/usr",
                "TMPDIR=/data/data/com.termux/files/usr/tmp",
                "TZ=UTC",
                "TERM=xterm",
            ],
            "Entrypoint": ["/entrypoint.sh"],
            "Cmd": ["login"],
            "WorkingDir": "/data/data/com.termux/files/home",
            "ArgsEscaped": True,
            "Shell": ["sh", "-c"],
        },
        "created": "2026-07-29T04:16:01.453185543Z",
        "os": "linux",
        "rootfs": {"type": "layers", "diff_ids": layers},
    },
    sort_keys=True,
    separators=(",", ":"),
).encode()
runtime_id = "sha256:" + hashlib.sha256(config).hexdigest()
runtime_tag = f"termux-mcp-qualified-runtime:{commit}"

packages = []
installed = []
for index, package in enumerate(("file", "jq", "python", "termux-services"), 1):
    version = f"1.{index}"
    architecture = "aarch64"
    packages.append(
        {
            "package": package,
            "version": version,
            "architecture": architecture,
            "fileName": f"{package}_{version}_{architecture}.deb",
            "sha256": f"{index:x}" * 64,
            "bytes": index,
        }
    )
    installed.append(
        {
            "package": package,
            "version": version,
            "architecture": architecture,
        }
    )
packages.sort(
    key=lambda item: (
        item["package"],
        item["version"],
        item["architecture"],
        item["fileName"],
    )
)
installed.sort(
    key=lambda item: (item["package"], item["version"], item["architecture"])
)
lock = {
    "schemaVersion": 1,
    "lockVersion": "1",
    "repository": "CyberBASSLord-666/termux-mcp-edge",
    "commit": commit,
    "androidRunId": run_id,
    "base": {
        "image": "termux/termux-docker:aarch64",
        "digest": base_digest,
        "imageId": base_id,
    },
    "requestedPackages": ["file", "jq", "python", "termux-services"],
    "resolution": {
        "resolver": "termux-apt-download-only",
        "repositoryMetadataAuthenticated": True,
        "packageBytesFrozenBeforeBuild": True,
        "finalImageBuildNetwork": "none",
    },
    "installation": {
        "method": "termux-dpkg-unpack-configure",
        "dependencyRepair": "none",
        "runtimeUser": "1000:1000",
    },
    "repositoryIndexes": [
        {"fileName": "packages.termux.dev_InRelease", "sha256": "e" * 64, "bytes": 1}
    ],
    "packages": packages,
}
lock_raw = (json.dumps(lock, sort_keys=True, separators=(",", ":")) + "\n").encode()
(root / "termux-runtime-package-lock-v1.json").write_bytes(lock_raw)

manifest = [
    {
        "Config": f"{runtime_id.removeprefix('sha256:')}.json",
        "RepoTags": [runtime_tag],
        "Layers": [
            f"runtime/{index}/layer.tar"
            for index in range(1, len(layer_raws) + 1)
        ],
    }
]
archive_path = root / "termux-qualified-runtime-image-v1.tar.gz"
with archive_path.open("wb") as raw:
    with gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as archive:
            members = [
                (f"{runtime_id.removeprefix('sha256:')}.json", config),
                *(
                    (f"runtime/{index}/layer.tar", layer_raw)
                    for index, layer_raw in enumerate(layer_raws, 1)
                ),
                (
                    "manifest.json",
                    json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode(),
                ),
            ]
            for name, data in members:
                info = tarfile.TarInfo(name)
                info.size = len(data)
                info.mode = 0o644
                info.mtime = 0
                archive.addfile(info, io.BytesIO(data))
archive_raw = archive_path.read_bytes()
inventory = "".join(
    f"{item['package']}\t{item['version']}\t{item['architecture']}\n"
    for item in installed
).encode()
snapshot = {
    "schemaVersion": 1,
    "snapshotVersion": "1",
    "status": "pass",
    "failureCode": None,
    "releaseQualificationEligible": False,
    "repository": "CyberBASSLord-666/termux-mcp-edge",
    "commit": commit,
    "androidRunId": run_id,
    "base": lock["base"],
    "runtimeImageId": runtime_id,
    "platform": {"os": "linux", "architecture": "arm64"},
    "rootfsLayers": layers,
    "packageLock": {
        "fileName": "termux-runtime-package-lock-v1.json",
        "sha256": hashlib.sha256(lock_raw).hexdigest(),
        "bytes": len(lock_raw),
    },
    "installedPackages": {
        "sha256": hashlib.sha256(inventory).hexdigest(),
        "count": len(installed),
        "packages": installed,
    },
    "archive": {
        "fileName": "termux-qualified-runtime-image-v1.tar.gz",
        "format": "docker-image-archive-v1",
        "compression": "gzip-no-name",
        "sha256": hashlib.sha256(archive_raw).hexdigest(),
        "bytes": len(archive_raw),
    },
    "claimBoundary": {
        "physicalDeviceObserved": False,
        "androidFrameworkObserved": False,
        "sustainedPhysicalSoak": False,
        "physicalCertification": "not_run",
    },
    "rebuildReproducibilityClaim": False,
}
(root / "termux-runtime-snapshot-v1.json").write_text(
    json.dumps(snapshot, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
aggregate = {
    "candidate": {"commit": commit, "androidRunId": run_id},
    "environment": {
        "image": "termux/termux-docker:aarch64",
        "imageDigest": base_digest,
        "rootfsImageId": base_id,
        "runtimeImageDigest": runtime_id,
    },
}
(root / "termux-native-aggregate-evidence-v4.json").write_text(
    json.dumps(aggregate, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
deployment = {
    "candidate": {"commit": commit, "nativeRunId": run_id},
    "environment": {
        "rootfsImage": "termux/termux-docker:aarch64",
        "rootfsDigest": base_digest,
        "rootfsImageId": base_id,
        "runtimeImageDigest": runtime_id,
        "androidLinker": {
            "observed": True,
            "path": "/system/bin/linker64",
            "sha256": "f" * 64,
            "bytes": 123,
        },
    },
}
(root / "automated-native-deployment-v1.json").write_text(
    json.dumps(deployment, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
(root / "fixture-env").write_text(
    "\n".join(
        (
            f"MOCK_RUNTIME_ID={runtime_id}",
            f"MOCK_RUNTIME_TAG={runtime_tag}",
            "MOCK_RUNTIME_LAYERS_JSON="
            + json.dumps(layers, separators=(",", ":")),
            "MOCK_RUNTIME_USER=1000:1000",
            "MOCK_RUNTIME_UID_GID=1000:1000",
            "MOCK_LINKER_SHA=" + "f" * 64,
            "MOCK_LINKER_BYTES=123",
            "MOCK_RUNTIME_PACKAGES="
            + inventory.decode().replace("\\", "\\\\").replace("\n", "\\n"),
            "MOCK_RUNTIME_PACKAGE_INPUTS="
            + "".join(
                f"{item['fileName']}\\t{item['sha256']}\\t{item['bytes']}\\n"
                for item in sorted(packages, key=lambda item: item["fileName"])
            ),
            "MOCK_RUNTIME_REPOSITORY_INDEXES="
            + "".join(
                f"{item['fileName']}\\t{item['sha256']}\\t{item['bytes']}\\n"
                for item in sorted(
                    lock["repositoryIndexes"], key=lambda item: item["fileName"]
                )
            ),
            "MOCK_RUNTIME_PACKAGE_LOCK="
            + "termux-runtime-package-lock-v1.json"
            + f"\\t{hashlib.sha256(lock_raw).hexdigest()}\\t{len(lock_raw)}\\n",
        )
    )
    + "\n",
    encoding="utf-8",
)

oci_root = root.parent / "oci-fixture"
oci_v29_root = root.parent / "oci-v29-fixture"
oci_root.mkdir(mode=0o700)
oci_v29_root.mkdir(mode=0o700)
for destination in (oci_root, oci_v29_root):
    for name in (
        "termux-runtime-package-lock-v1.json",
        "termux-native-aggregate-evidence-v4.json",
        "automated-native-deployment-v1.json",
        "fixture-env",
    ):
        (destination / name).write_bytes((root / name).read_bytes())

config_name = f"blobs/sha256/{runtime_id.removeprefix('sha256:')}"
layer_blobs = []
layer_names = []
layer_descriptors = []
for layer_raw, diff_id in zip(layer_raws, layers):
    layer_blob = layer_raw
    layer_blob_digest = diff_id.removeprefix("sha256:")
    layer_blobs.append(layer_blob)
    layer_names.append(f"blobs/sha256/{layer_blob_digest}")
    layer_descriptors.append(
        {
            "mediaType": "application/vnd.oci.image.layer.v1.tar",
            "digest": f"sha256:{layer_blob_digest}",
            "size": len(layer_blob),
        }
    )
oci_manifest = {
    "schemaVersion": 2,
    "mediaType": "application/vnd.oci.image.manifest.v1+json",
    "config": {
        "mediaType": "application/vnd.oci.image.config.v1+json",
        "digest": runtime_id,
        "size": len(config),
    },
    "layers": layer_descriptors,
}
oci_manifest_raw = json.dumps(
    oci_manifest, sort_keys=True, separators=(",", ":")
).encode()
oci_manifest_digest = hashlib.sha256(oci_manifest_raw).hexdigest()
oci_manifest_name = f"blobs/sha256/{oci_manifest_digest}"
oci_index = {
    "schemaVersion": 2,
    "mediaType": "application/vnd.oci.image.index.v1+json",
    "manifests": [
        {
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "digest": f"sha256:{oci_manifest_digest}",
            "size": len(oci_manifest_raw),
            "annotations": {
                "io.containerd.image.name": f"docker.io/library/{runtime_tag}",
                "org.opencontainers.image.ref.name": commit,
            },
        }
    ],
}
legacy_manifest = [
    {
        "Config": config_name,
        "RepoTags": [runtime_tag],
        "Layers": layer_names,
        "LayerSources": {
            diff_id: {
                "mediaType": descriptor["mediaType"],
                "size": descriptor["size"],
                "digest": descriptor["digest"],
            }
            for diff_id, descriptor in zip(layers, layer_descriptors)
        },
    }
]
config_fields_v28 = [
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
config_fields_v29 = [
    (name, default, True if name == "OnBuild" else omit)
    for name, default, omit in config_fields_v28
    if name != "MacAddress"
]

def go_json(value):
    text = json.dumps(
        value,
        ensure_ascii=False,
        allow_nan=False,
        separators=(",", ":"),
    )
    return (
        text.replace("&", r"\u0026")
        .replace("<", r"\u003c")
        .replace(">", r"\u003e")
        .replace("\u2028", r"\u2028")
        .replace("\u2029", r"\u2029")
        .encode()
    )

def omitted(name, value):
    if name in {"ExposedPorts", "OnBuild", "Shell"}:
        return value is None or value == {} or value == []
    if name in {"Healthcheck", "StopTimeout"}:
        return value is None
    if name in {"ArgsEscaped", "NetworkDisabled"}:
        return value is False
    if name in {"MacAddress", "StopSignal"}:
        return value == ""
    return False

def go_config_raw(sparse, fields):
    allowed = {name for name, _default, _omit in fields}
    if set(sparse) - allowed:
        raise SystemExit("fixture runtime config unsupported")
    values = {name: default for name, default, _omit in fields}
    values.update(sparse)
    pairs = []
    for name, _default, omit_empty in fields:
        value = values[name]
        if omit_empty and omitted(name, value):
            continue
        pairs.append(go_json(name) + b":" + go_json(value))
    return b"{" + b",".join(pairs) + b"}"

def sorted_raw_map(fields):
    return b"{" + b",".join(
        go_json(key) + b":" + fields[key]
        for key in sorted(fields)
    ) + b"}"

def moby_legacy_records(diff_ids, image_config, fields):
    chain_ids = []
    for diff_id in diff_ids:
        chain_id = (
            diff_id
            if not chain_ids
            else "sha256:" + hashlib.sha256(
                f"{chain_ids[-1]} {diff_id}".encode()
            ).hexdigest()
        )
        chain_ids.append(chain_id)
    result = []
    parent_id = None
    for index, chain_id in enumerate(chain_ids):
        terminal = index == len(chain_ids) - 1
        record_created = (
            image_config["created"]
            if terminal
            else "1970-01-01T00:00:00Z"
        )
        create_fields = {
            "container_config": go_config_raw({}, fields),
            "created": go_json(record_created),
            "layer_id": go_json(chain_id),
        }
        if parent_id is not None:
            create_fields["parent"] = go_json(f"sha256:{parent_id}")
        if terminal:
            create_fields["config"] = go_config_raw(
                image_config["config"],
                fields,
            )
            create_fields["architecture"] = go_json(
                image_config["architecture"]
            )
            create_fields["os"] = go_json(image_config["os"])
        image_id = hashlib.sha256(
            sorted_raw_map(create_fields)
        ).hexdigest()
        record_pairs = [("id", go_json(image_id))]
        if parent_id is not None:
            record_pairs.append(("parent", go_json(parent_id)))
        record_pairs.extend(
            [
                ("created", go_json(record_created)),
                ("container_config", go_config_raw({}, fields)),
            ]
        )
        if terminal:
            record_pairs.extend(
                [
                    (
                        "config",
                        go_config_raw(image_config["config"], fields),
                    ),
                    (
                        "architecture",
                        go_json(image_config["architecture"]),
                    ),
                ]
            )
        record_pairs.append(("os", go_json(image_config["os"])))
        raw = b"{" + b",".join(
            go_json(key) + b":" + value
            for key, value in record_pairs
        ) + b"}"
        result.append(
            ("blobs/sha256/" + hashlib.sha256(raw).hexdigest(), raw)
        )
        parent_id = image_id
    return result

config_doc = json.loads(config)
legacy_metadata_v28 = moby_legacy_records(
    layers,
    config_doc,
    config_fields_v28,
)
legacy_metadata_v29 = moby_legacy_records(
    layers,
    config_doc,
    config_fields_v29,
)
expected_v28_ids = [
    "cf0ede2642e424d4d48cb8015214e299baa4c1dec750f806cfbd8e1d7b626360",
    "a9bde2bf224ae50639f63b3da61e069e83683acfdb18bffdb61a727b8811e06a",
    "782d853bf1f461852b7b992a10e4938a38a44218ceb7086d232f730877985de9",
    "492ac3ebfe5641c7b3a30d5e4770cf4dba611a56c256008e7c52eb36e936e592",
    "323adfa5f234e043c7a9e205710d52c26efc77ba888a78cc9e2a051d8e8bc1eb",
    "704e4583718ad44243c24a3b61c7a1989b0ac091e61c4424d4a7ec8e53ea5749",
]
expected_v29_ids = [
    "f17e41d764ef10d85df93dfd3447177e044a4cc3f7744760d5b52c00b3eb57ea",
    "3f61b42f2afd0fb92c6a8b654957a0872ef1fdc95b65f02c51e77f211bd6326a",
    "6df0ca6d709be3a75309597420b9714765ccf15ff6f06ab7571937d55bd7dbe9",
    "5dd4505c4d244bab2426bde5aeffe3cbcf5bb0a10e5bb9905a062407340b0431",
    "840a0f2762ad23fdd3371e008c49927a6fcf607d86da56e5c32b0d1fe811ccc1",
    "6d11f415dfeae2c9fc0c549003ebb49f34faf5e734c843a1cd737fb70cc4786c",
]
if [
    json.loads(raw)["id"]
    for _name, raw in legacy_metadata_v28
] != expected_v28_ids:
    raise SystemExit("Moby v28 CreateID fixture oracle mismatch")
if [
    json.loads(raw)["id"]
    for _name, raw in legacy_metadata_v29
] != expected_v29_ids:
    raise SystemExit("Docker 29 CreateID fixture oracle mismatch")
repositories = {
    "termux-mcp-qualified-runtime": {
        commit: layer_descriptors[-1]["digest"].removeprefix("sha256:")
    }
}

def write_oci_fixture(destination, metadata_records):
    archive_path = (
        destination / "termux-qualified-runtime-image-v1.tar.gz"
    )
    with archive_path.open("wb") as raw:
        with gzip.GzipFile(
            fileobj=raw,
            mode="wb",
            filename="",
            mtime=0,
        ) as compressed:
            with tarfile.open(fileobj=compressed, mode="w") as archive:
                for directory in ("blobs/", "blobs/sha256/"):
                    info = tarfile.TarInfo(directory)
                    info.type = tarfile.DIRTYPE
                    info.mode = 0o755
                    info.mtime = 0
                    archive.addfile(info)
                members = [
                    (
                        "oci-layout",
                        json.dumps(
                            {"imageLayoutVersion": "1.0.0"},
                            sort_keys=True,
                            separators=(",", ":"),
                        ).encode(),
                    ),
                    (
                        "index.json",
                        json.dumps(
                            oci_index,
                            sort_keys=True,
                            separators=(",", ":"),
                        ).encode(),
                    ),
                    (oci_manifest_name, oci_manifest_raw),
                    (config_name, config),
                    *zip(layer_names, layer_blobs),
                    *metadata_records,
                    (
                        "manifest.json",
                        json.dumps(
                            legacy_manifest,
                            sort_keys=True,
                            separators=(",", ":"),
                        ).encode(),
                    ),
                    (
                        "repositories",
                        json.dumps(
                            repositories,
                            sort_keys=True,
                            separators=(",", ":"),
                        ).encode(),
                    ),
                ]
                for name, data in members:
                    info = tarfile.TarInfo(name)
                    info.size = len(data)
                    info.mode = 0o644
                    info.mtime = 0
                    archive.addfile(info, io.BytesIO(data))
    archive_raw = archive_path.read_bytes()
    oci_snapshot = json.loads(json.dumps(snapshot))
    oci_snapshot["archive"]["sha256"] = hashlib.sha256(
        archive_raw
    ).hexdigest()
    oci_snapshot["archive"]["bytes"] = len(archive_raw)
    (destination / "termux-runtime-snapshot-v1.json").write_text(
        json.dumps(
            oci_snapshot,
            sort_keys=True,
            separators=(",", ":"),
        )
        + "\n",
        encoding="utf-8",
    )

write_oci_fixture(oci_root, legacy_metadata_v28)
write_oci_fixture(oci_v29_root, legacy_metadata_v29)
PY

while IFS='=' read -r key value; do
  export "$key=$value"
done <"$TMP/fixture/fixture-env"
export MOCK_DOCKER_STATE="$TMP/docker-loaded"
export TERMUX_MCP_TEST_FAKE_ARM64=1
export PYTHONPATH="$TMP/site"
export PATH="$TMP/bin:$PATH"

run_verifier() {
  local fixture="$1"
  local output="$2"
  bash "$VERIFIER" \
    --archive "$fixture/termux-qualified-runtime-image-v1.tar.gz" \
    --package-lock "$fixture/termux-runtime-package-lock-v1.json" \
    --snapshot-manifest "$fixture/termux-runtime-snapshot-v1.json" \
    --aggregate-evidence "$fixture/termux-native-aggregate-evidence-v4.json" \
    --deployment-evidence "$fixture/automated-native-deployment-v1.json" \
    --output "$output"
}

mkdir -m 700 "$TMP/success"
run_verifier \
  "$TMP/fixture" \
  "$TMP/success/termux-runtime-snapshot-replay-v1.json" \
  >"$TMP/success.log"
REPORT="$TMP/success/termux-runtime-snapshot-replay-v1.json"
jq -e \
  --arg runtime "$MOCK_RUNTIME_ID" \
  --arg linker "$MOCK_LINKER_SHA" '
    (keys == [
      "androidLinker","claimBoundary","commit","failureCode",
      "installedPackages","packageLock","rebuildReproducibilityClaim",
      "releaseQualificationEligible","replayVersion","repository","runtimeImageId",
      "schemaVersion","snapshot","status","verification"
    ])
    and .schemaVersion == 1
    and .replayVersion == "1"
    and .status == "pass"
    and .failureCode == null
    and .releaseQualificationEligible == false
    and .runtimeImageId == $runtime
    and .androidLinker.sha256 == $linker
    and .verification == {
      archiveDigestVerified:true,
      singleImageArchive:true,
      loadedImageIdVerified:true,
      platformVerified:true,
      runtimeUserVerified:true,
      rootfsLayersVerified:true,
      packageLockVerified:true,
      packageInputBytesVerified:true,
      repositoryIndexBytesVerified:true,
      installedPackageInventoryVerified:true,
      requiredRuntimeCommandsVerified:true,
      androidLinkerVerified:true,
      runtimeNetworkAccess:false
    }
    and .claimBoundary == {
      physicalDeviceObserved:false,
      androidFrameworkObserved:false,
      sustainedPhysicalSoak:false,
      physicalCertification:"not_run"
    }
    and .rebuildReproducibilityClaim == false
  ' "$REPORT" >/dev/null || fail valid_replay_report_invalid
[[ ! -e "$MOCK_DOCKER_STATE" ]] || fail replay_did_not_remove_loaded_image

mkdir -m 700 "$TMP/oci-success"
run_verifier \
  "$TMP/oci-fixture" \
  "$TMP/oci-success/termux-runtime-snapshot-replay-v1.json" \
  >"$TMP/oci-success.log"
jq -e \
  --arg runtime "$MOCK_RUNTIME_ID" '
    .status == "pass"
    and .runtimeImageId == $runtime
    and .verification.singleImageArchive == true
    and .verification.runtimeUserVerified == true
    and .verification.rootfsLayersVerified == true
    and .verification.runtimeNetworkAccess == false
  ' "$TMP/oci-success/termux-runtime-snapshot-replay-v1.json" >/dev/null \
  || fail valid_oci_replay_report_invalid
[[ ! -e "$MOCK_DOCKER_STATE" ]] || fail oci_replay_did_not_remove_loaded_image

mkdir -m 700 "$TMP/oci-v29-success"
run_verifier \
  "$TMP/oci-v29-fixture" \
  "$TMP/oci-v29-success/termux-runtime-snapshot-replay-v1.json" \
  >"$TMP/oci-v29-success.log"
jq -e \
  --arg runtime "$MOCK_RUNTIME_ID" '
    .status == "pass"
    and .runtimeImageId == $runtime
    and .verification.singleImageArchive == true
    and .verification.platformVerified == true
    and .verification.runtimeNetworkAccess == false
  ' "$TMP/oci-v29-success/termux-runtime-snapshot-replay-v1.json" >/dev/null \
  || fail valid_oci_v29_replay_report_invalid
[[ ! -e "$MOCK_DOCKER_STATE" ]] || fail oci_v29_replay_did_not_remove_loaded_image

expect_failure() {
  local name="$1"
  local fixture="$2"
  mkdir -m 700 "$TMP/$name-output"
  rm -f -- "$MOCK_DOCKER_STATE"
  if run_verifier \
    "$fixture" \
    "$TMP/$name-output/termux-runtime-snapshot-replay-v1.json" \
    >"$TMP/$name.log" 2>&1
  then
    fail "$name was accepted"
  fi
  [[ ! -e "$TMP/$name-output/termux-runtime-snapshot-replay-v1.json" ]] \
    || fail "$name published a replay report"
  [[ ! -e "$MOCK_DOCKER_STATE" ]] \
    || fail "$name left a loaded image behind"
}

mutate_archive_fixture() {
  local source="$1" destination="$2" operation="$3"
  cp -a -- "$source" "$destination"
  python3 - \
    "$destination/termux-qualified-runtime-image-v1.tar.gz" \
    "$destination/termux-runtime-snapshot-v1.json" \
    "$operation" <<'PY'
import gzip
import hashlib
import io
import json
import pathlib
import tarfile
import sys

archive_path = pathlib.Path(sys.argv[1])
snapshot_path = pathlib.Path(sys.argv[2])
operation = sys.argv[3]
entries = []
runtime_id = None
with tarfile.open(archive_path, mode="r:gz") as archive:
    for member in archive.getmembers():
        if member.isfile():
            source = archive.extractfile(member)
            if source is None:
                raise SystemExit("fixture archive member unreadable")
            data = source.read()
        elif member.isdir():
            data = None
        else:
            raise SystemExit("unexpected fixture archive member type")
        entries.append([member.name, data, member.mode])

def file_index(name):
    matches = [index for index, entry in enumerate(entries) if entry[0] == name]
    if len(matches) != 1:
        raise SystemExit(f"fixture member lookup failed: {name}")
    return matches[0]

def oci_metadata_indexes():
    index_value = json.loads(entries[file_index("index.json")][1])
    manifest_digest = index_value["manifests"][0]["digest"].removeprefix("sha256:")
    manifest_name = f"blobs/sha256/{manifest_digest}"
    manifest_value = json.loads(entries[file_index(manifest_name)][1])
    referenced = {
        manifest_name,
        "blobs/sha256/" + manifest_value["config"]["digest"].removeprefix("sha256:"),
        *(
            "blobs/sha256/" + descriptor["digest"].removeprefix("sha256:")
            for descriptor in manifest_value["layers"]
        ),
    }
    return [
        index
        for index, entry in enumerate(entries)
        if entry[1] is not None
        and entry[0].startswith("blobs/sha256/")
        and entry[0] not in referenced
    ]

def replace_content_addressed_json(index, value):
    raw = json.dumps(value, separators=(",", ":")).encode()
    entries[index][0] = "blobs/sha256/" + hashlib.sha256(raw).hexdigest()
    entries[index][1] = raw

def oci_metadata_records():
    indexes = oci_metadata_indexes()
    if len(indexes) != 6:
        raise SystemExit("fixture legacy metadata lookup failed")
    return [
        (index, json.loads(entries[index][1]))
        for index in indexes
    ]

def oci_metadata_chain():
    records = oci_metadata_records()
    roots = [
        item
        for item in records
        if "parent" not in item[1]
    ]
    if len(roots) != 1:
        raise SystemExit("fixture legacy metadata root lookup failed")
    children = {}
    for item in records:
        parent = item[1].get("parent")
        if parent is not None:
            if parent in children:
                raise SystemExit("fixture legacy metadata branch found")
            children[parent] = item
    ordered = []
    current = roots[0]
    while True:
        ordered.append(current)
        child = children.get(current[1]["id"])
        if child is None:
            break
        current = child
    if len(ordered) != len(records):
        raise SystemExit("fixture legacy metadata chain incomplete")
    return ordered

if operation == "unreferenced":
    entries.append(["unreferenced.bin", b"unreferenced", 0o644])
elif operation == "unsafe":
    entries.append(["../escape", b"escape", 0o644])
elif operation == "oci-generic-compressed":
    metadata_indexes = oci_metadata_indexes()
    index_index = file_index("index.json")
    index_value = json.loads(entries[index_index][1])
    old_manifest_digest = (
        index_value["manifests"][0]["digest"].removeprefix("sha256:")
    )
    manifest_index = file_index(f"blobs/sha256/{old_manifest_digest}")
    manifest_value = json.loads(entries[manifest_index][1])
    compressed_names = []
    for descriptor in manifest_value["layers"]:
        old_name = (
            "blobs/sha256/"
            + descriptor["digest"].removeprefix("sha256:")
        )
        layer_index = file_index(old_name)
        buffer = io.BytesIO()
        with gzip.GzipFile(
            fileobj=buffer,
            mode="wb",
            filename="",
            mtime=0,
        ) as compressed:
            compressed.write(entries[layer_index][1])
        compressed_raw = buffer.getvalue()
        compressed_digest = hashlib.sha256(compressed_raw).hexdigest()
        compressed_name = f"blobs/sha256/{compressed_digest}"
        entries[layer_index][0] = compressed_name
        entries[layer_index][1] = compressed_raw
        descriptor["mediaType"] = (
            "application/vnd.oci.image.layer.v1.tar+gzip"
        )
        descriptor["digest"] = f"sha256:{compressed_digest}"
        descriptor["size"] = len(compressed_raw)
        compressed_names.append(compressed_name)
    manifest_raw = json.dumps(
        manifest_value, sort_keys=True, separators=(",", ":")
    ).encode()
    manifest_digest = hashlib.sha256(manifest_raw).hexdigest()
    entries[manifest_index][0] = f"blobs/sha256/{manifest_digest}"
    entries[manifest_index][1] = manifest_raw
    index_value["manifests"][0]["digest"] = f"sha256:{manifest_digest}"
    index_value["manifests"][0]["size"] = len(manifest_raw)
    entries[index_index][1] = json.dumps(
        index_value, sort_keys=True, separators=(",", ":")
    ).encode()
    legacy_index = file_index("manifest.json")
    legacy_value = json.loads(entries[legacy_index][1])
    legacy_value[0]["Layers"] = compressed_names
    del legacy_value[0]["LayerSources"]
    entries[legacy_index][1] = json.dumps(
        legacy_value, sort_keys=True, separators=(",", ":")
    ).encode()
    repositories_index = file_index("repositories")
    repositories_value = json.loads(entries[repositories_index][1])
    repository = next(iter(repositories_value))
    tag = next(iter(repositories_value[repository]))
    repositories_value[repository][tag] = (
        manifest_value["layers"][-1]["digest"].removeprefix("sha256:")
    )
    entries[repositories_index][1] = json.dumps(
        repositories_value, sort_keys=True, separators=(",", ":")
    ).encode()
    for metadata_index in sorted(metadata_indexes, reverse=True):
        del entries[metadata_index]
elif operation == "oci-platform":
    index = file_index("index.json")
    value = json.loads(entries[index][1])
    value["manifests"][0]["platform"] = {
        "architecture": "amd64",
        "os": "linux",
    }
    entries[index][1] = json.dumps(
        value, sort_keys=True, separators=(",", ":")
    ).encode()
elif operation == "oci-layer-sources-mismatch":
    manifest_index = file_index("manifest.json")
    value = json.loads(entries[manifest_index][1])
    source = next(iter(value[0]["LayerSources"].values()))
    source["size"] += 1
    entries[manifest_index][1] = json.dumps(
        value, sort_keys=True, separators=(",", ":")
    ).encode()
elif operation == "oci-layer-sources-extra":
    manifest_index = file_index("manifest.json")
    value = json.loads(entries[manifest_index][1])
    source = dict(next(iter(value[0]["LayerSources"].values())))
    value[0]["LayerSources"]["sha256:" + "0" * 64] = source
    entries[manifest_index][1] = json.dumps(
        value, sort_keys=True, separators=(",", ":")
    ).encode()
elif operation == "oci-layer-sources-float":
    manifest_index = file_index("manifest.json")
    value = json.loads(entries[manifest_index][1])
    source = next(iter(value[0]["LayerSources"].values()))
    source["size"] = float(source["size"])
    entries[manifest_index][1] = json.dumps(
        value, sort_keys=True, separators=(",", ":")
    ).encode()
elif operation == "oci-layer-sources-missing":
    manifest_index = file_index("manifest.json")
    value = json.loads(entries[manifest_index][1])
    del value[0]["LayerSources"]
    entries[manifest_index][1] = json.dumps(
        value, sort_keys=True, separators=(",", ":")
    ).encode()
elif operation == "oci-repositories-mismatch":
    repositories_index = file_index("repositories")
    value = json.loads(entries[repositories_index][1])
    repository = next(iter(value))
    tag = next(iter(value[repository]))
    value[repository][tag] = "0" * 64
    entries[repositories_index][1] = json.dumps(
        value, sort_keys=True, separators=(",", ":")
    ).encode()
elif operation == "oci-legacy-metadata-substitution":
    records = oci_metadata_records()
    entries[records[0][0]][1] += b" "
elif operation == "oci-legacy-metadata-extra-key":
    records = oci_metadata_records()
    index, value = records[0]
    value["unexpected"] = True
    replace_content_addressed_json(index, value)
elif operation == "oci-legacy-metadata-cycle":
    records = oci_metadata_records()
    index, value = next(
        (index, value)
        for index, value in records
        if value.get("architecture") == "arm64"
    )
    value["parent"] = value["id"]
    replace_content_addressed_json(index, value)
elif operation == "oci-legacy-metadata-fork":
    records = oci_metadata_chain()
    root = records[0][1]
    index, value = records[2]
    value["parent"] = root["id"]
    replace_content_addressed_json(index, value)
elif operation == "oci-legacy-metadata-created":
    records = oci_metadata_chain()
    index, value = records[2]
    value["created"] = "2026-07-29T04:16:01.453185543Z"
    replace_content_addressed_json(index, value)
elif operation == "oci-legacy-metadata-config":
    records = oci_metadata_records()
    index, value = next(
        (index, value)
        for index, value in records
        if value.get("architecture") == "arm64"
    )
    value["config"]["User"] = "0:0"
    replace_content_addressed_json(index, value)
elif operation == "oci-legacy-metadata-duplicate-id":
    records = oci_metadata_chain()
    index, value = records[2]
    value["id"] = records[1][1]["id"]
    replace_content_addressed_json(index, value)
elif operation == "oci-legacy-metadata-rewrite-chain":
    records = oci_metadata_chain()
    replacement_ids = [
        f"{index:x}" * 64
        for index in range(10, 10 + len(records))
    ]
    for position, (index, value) in enumerate(records):
        value["id"] = replacement_ids[position]
        if position:
            value["parent"] = replacement_ids[position - 1]
        replace_content_addressed_json(index, value)
elif operation == "oci-legacy-metadata-bool-int":
    records = oci_metadata_chain()
    index, value = records[0]
    value["container_config"]["AttachStdin"] = 0
    replace_content_addressed_json(index, value)
elif operation == "oci-legacy-metadata-extra":
    records = oci_metadata_records()
    value = dict(records[0][1])
    value["id"] = "7" * 64
    raw = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    entries.append(
        [
            "blobs/sha256/" + hashlib.sha256(raw).hexdigest(),
            raw,
            0o644,
        ]
    )
elif operation == "oci-manifest-substitution":
    index_value = json.loads(entries[file_index("index.json")][1])
    digest = index_value["manifests"][0]["digest"].removeprefix("sha256:")
    manifest = file_index(f"blobs/sha256/{digest}")
    entries[manifest][1] += b" "
elif operation == "classic-layer-substitution":
    manifest_value = json.loads(entries[file_index("manifest.json")][1])
    layer = file_index(manifest_value[0]["Layers"][0])
    entries[layer][1] += b" "
elif operation == "classic-root-user":
    manifest_index = file_index("manifest.json")
    manifest_value = json.loads(entries[manifest_index][1])
    config_index = file_index(manifest_value[0]["Config"])
    config_value = json.loads(entries[config_index][1])
    config_value["config"]["User"] = "0:0"
    config_raw = json.dumps(
        config_value, sort_keys=True, separators=(",", ":")
    ).encode()
    runtime_id = "sha256:" + hashlib.sha256(config_raw).hexdigest()
    config_name = f"{runtime_id.removeprefix('sha256:')}.json"
    entries[config_index][0] = config_name
    entries[config_index][1] = config_raw
    manifest_value[0]["Config"] = config_name
    entries[manifest_index][1] = json.dumps(
        manifest_value, sort_keys=True, separators=(",", ":")
    ).encode()
elif operation == "oci-root-user":
    index_index = file_index("index.json")
    index_value = json.loads(entries[index_index][1])
    old_manifest_digest = index_value["manifests"][0]["digest"]
    manifest_index = file_index(
        f"blobs/sha256/{old_manifest_digest.removeprefix('sha256:')}"
    )
    manifest_value = json.loads(entries[manifest_index][1])
    old_config_digest = manifest_value["config"]["digest"]
    config_index = file_index(
        f"blobs/sha256/{old_config_digest.removeprefix('sha256:')}"
    )
    config_value = json.loads(entries[config_index][1])
    config_value["config"]["User"] = "0:0"
    config_raw = json.dumps(
        config_value, sort_keys=True, separators=(",", ":")
    ).encode()
    runtime_id = "sha256:" + hashlib.sha256(config_raw).hexdigest()
    config_name = f"blobs/sha256/{runtime_id.removeprefix('sha256:')}"
    entries[config_index][0] = config_name
    entries[config_index][1] = config_raw
    manifest_value["config"]["digest"] = runtime_id
    manifest_value["config"]["size"] = len(config_raw)
    manifest_raw = json.dumps(
        manifest_value, sort_keys=True, separators=(",", ":")
    ).encode()
    manifest_digest = "sha256:" + hashlib.sha256(manifest_raw).hexdigest()
    entries[manifest_index][0] = (
        f"blobs/sha256/{manifest_digest.removeprefix('sha256:')}"
    )
    entries[manifest_index][1] = manifest_raw
    index_value["manifests"][0]["digest"] = manifest_digest
    index_value["manifests"][0]["size"] = len(manifest_raw)
    entries[index_index][1] = json.dumps(
        index_value, sort_keys=True, separators=(",", ":")
    ).encode()
    legacy_index = file_index("manifest.json")
    legacy_value = json.loads(entries[legacy_index][1])
    legacy_value[0]["Config"] = config_name
    entries[legacy_index][1] = json.dumps(
        legacy_value, sort_keys=True, separators=(",", ":")
    ).encode()
else:
    raise SystemExit(f"unknown fixture archive mutation: {operation}")

with archive_path.open("wb") as raw:
    with gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as archive:
            for name, data, mode in entries:
                info = tarfile.TarInfo(name)
                info.mode = mode
                info.mtime = 0
                if data is None:
                    info.type = tarfile.DIRTYPE
                    archive.addfile(info)
                else:
                    info.size = len(data)
                    archive.addfile(info, io.BytesIO(data))

archive_raw = archive_path.read_bytes()
snapshot = json.loads(snapshot_path.read_text(encoding="utf-8"))
snapshot["archive"]["sha256"] = hashlib.sha256(archive_raw).hexdigest()
snapshot["archive"]["bytes"] = len(archive_raw)
if runtime_id is not None:
    snapshot["runtimeImageId"] = runtime_id
snapshot_path.write_text(
    json.dumps(snapshot, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
if runtime_id is not None:
    for file_name in (
        "termux-native-aggregate-evidence-v4.json",
        "automated-native-deployment-v1.json",
    ):
        path = archive_path.parent / file_name
        value = json.loads(path.read_text(encoding="utf-8"))
        value["environment"]["runtimeImageDigest"] = runtime_id
        path.write_text(
            json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
PY
  chmod 600 \
    "$destination/termux-qualified-runtime-image-v1.tar.gz" \
    "$destination/termux-runtime-snapshot-v1.json"
}

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-generic-compressed" \
  oci-generic-compressed
mkdir -m 700 "$TMP/oci-generic-compressed-output"
run_verifier \
  "$TMP/oci-generic-compressed" \
  "$TMP/oci-generic-compressed-output/termux-runtime-snapshot-replay-v1.json" \
  >"$TMP/oci-generic-compressed.log"
jq -e \
  --arg runtime "$MOCK_RUNTIME_ID" '
    .status == "pass"
    and .runtimeImageId == $runtime
    and .verification.singleImageArchive == true
    and .verification.rootfsLayersVerified == true
    and .verification.runtimeNetworkAccess == false
  ' "$TMP/oci-generic-compressed-output/termux-runtime-snapshot-replay-v1.json" \
  >/dev/null || fail valid_compressed_oci_replay_report_invalid
[[ ! -e "$MOCK_DOCKER_STATE" ]] \
  || fail compressed_oci_replay_did_not_remove_loaded_image

cp -a "$TMP/fixture" "$TMP/archive-substitution"
printf X >>"$TMP/archive-substitution/termux-qualified-runtime-image-v1.tar.gz"
expect_failure archive_substitution "$TMP/archive-substitution"

mutate_archive_fixture \
  "$TMP/fixture" "$TMP/classic-layer-substitution" classic-layer-substitution
expect_failure classic_layer_substitution "$TMP/classic-layer-substitution"

mutate_archive_fixture \
  "$TMP/fixture" "$TMP/classic-root-user" classic-root-user
expect_failure classic_root_user "$TMP/classic-root-user"
grep -Fq 'ERROR: archive_config_invalid' "$TMP/classic_root_user.log" \
  || fail classic_root_user_failed_for_wrong_reason

mutate_archive_fixture \
  "$TMP/fixture" "$TMP/unreferenced-member" unreferenced
expect_failure unreferenced_member "$TMP/unreferenced-member"

mutate_archive_fixture \
  "$TMP/fixture" "$TMP/unsafe-member" unsafe
expect_failure unsafe_member "$TMP/unsafe-member"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-platform" oci-platform
expect_failure oci_platform "$TMP/oci-platform"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-layer-sources-mismatch" \
  oci-layer-sources-mismatch
expect_failure oci_layer_sources_mismatch "$TMP/oci-layer-sources-mismatch"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-layer-sources-extra" \
  oci-layer-sources-extra
expect_failure oci_layer_sources_extra "$TMP/oci-layer-sources-extra"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-layer-sources-float" \
  oci-layer-sources-float
expect_failure oci_layer_sources_float "$TMP/oci-layer-sources-float"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-layer-sources-missing" \
  oci-layer-sources-missing
expect_failure oci_layer_sources_missing "$TMP/oci-layer-sources-missing"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-repositories-mismatch" \
  oci-repositories-mismatch
expect_failure oci_repositories_mismatch "$TMP/oci-repositories-mismatch"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-legacy-metadata-substitution" \
  oci-legacy-metadata-substitution
expect_failure \
  oci_legacy_metadata_substitution \
  "$TMP/oci-legacy-metadata-substitution"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-legacy-metadata-extra-key" \
  oci-legacy-metadata-extra-key
expect_failure \
  oci_legacy_metadata_extra_key \
  "$TMP/oci-legacy-metadata-extra-key"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-legacy-metadata-cycle" \
  oci-legacy-metadata-cycle
expect_failure oci_legacy_metadata_cycle "$TMP/oci-legacy-metadata-cycle"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-legacy-metadata-fork" \
  oci-legacy-metadata-fork
expect_failure oci_legacy_metadata_fork "$TMP/oci-legacy-metadata-fork"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-legacy-metadata-created" \
  oci-legacy-metadata-created
expect_failure oci_legacy_metadata_created "$TMP/oci-legacy-metadata-created"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-legacy-metadata-config" \
  oci-legacy-metadata-config
expect_failure oci_legacy_metadata_config "$TMP/oci-legacy-metadata-config"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-legacy-metadata-duplicate-id" \
  oci-legacy-metadata-duplicate-id
expect_failure \
  oci_legacy_metadata_duplicate_id \
  "$TMP/oci-legacy-metadata-duplicate-id"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-legacy-metadata-rewrite-chain" \
  oci-legacy-metadata-rewrite-chain
expect_failure \
  oci_legacy_metadata_rewrite_chain \
  "$TMP/oci-legacy-metadata-rewrite-chain"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-legacy-metadata-bool-int" \
  oci-legacy-metadata-bool-int
expect_failure \
  oci_legacy_metadata_bool_int \
  "$TMP/oci-legacy-metadata-bool-int"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-legacy-metadata-extra" \
  oci-legacy-metadata-extra
expect_failure oci_legacy_metadata_extra "$TMP/oci-legacy-metadata-extra"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-manifest-substitution" oci-manifest-substitution
expect_failure oci_manifest_substitution "$TMP/oci-manifest-substitution"

mutate_archive_fixture \
  "$TMP/oci-fixture" "$TMP/oci-root-user" oci-root-user
expect_failure oci_root_user "$TMP/oci-root-user"
grep -Fq 'ERROR: archive_config_invalid' "$TMP/oci_root_user.log" \
  || fail oci_root_user_failed_for_wrong_reason

cp -a "$TMP/fixture" "$TMP/duplicate-key"
python3 - "$TMP/duplicate-key/termux-runtime-package-lock-v1.json" <<'PY'
import pathlib
import sys
path = pathlib.Path(sys.argv[1])
raw = path.read_text(encoding="utf-8")
path.write_text(raw.replace('{"androidRunId":', '{"schemaVersion":1,"androidRunId":', 1), encoding="utf-8")
PY
expect_failure duplicate_key "$TMP/duplicate-key"

cp -a "$TMP/fixture" "$TMP/wrong-runtime"
jq -cS '.environment.runtimeImageDigest = ("sha256:" + ("0" * 64))' \
  "$TMP/wrong-runtime/termux-native-aggregate-evidence-v4.json" \
  >"$TMP/wrong-runtime/aggregate.tmp"
mv "$TMP/wrong-runtime/aggregate.tmp" \
  "$TMP/wrong-runtime/termux-native-aggregate-evidence-v4.json"
expect_failure wrong_runtime "$TMP/wrong-runtime"

original_packages="$MOCK_RUNTIME_PACKAGES"
MOCK_RUNTIME_PACKAGES="${MOCK_RUNTIME_PACKAGES/1.1/9.9}"
export MOCK_RUNTIME_PACKAGES
expect_failure package_drift "$TMP/fixture"
grep -Fq 'ERROR: runtime_package_installation_mismatch' \
  "$TMP/package_drift.log" \
  || fail package_drift_failed_for_wrong_reason
MOCK_RUNTIME_PACKAGES="$original_packages"
export MOCK_RUNTIME_PACKAGES

original_package_inputs="$MOCK_RUNTIME_PACKAGE_INPUTS"
MOCK_RUNTIME_PACKAGE_INPUTS="${MOCK_RUNTIME_PACKAGE_INPUTS/11111111/00000000}"
export MOCK_RUNTIME_PACKAGE_INPUTS
expect_failure package_input_drift "$TMP/fixture"
MOCK_RUNTIME_PACKAGE_INPUTS="$original_package_inputs"
export MOCK_RUNTIME_PACKAGE_INPUTS

original_repository_indexes="$MOCK_RUNTIME_REPOSITORY_INDEXES"
MOCK_RUNTIME_REPOSITORY_INDEXES="${MOCK_RUNTIME_REPOSITORY_INDEXES/eeeeeeee/00000000}"
export MOCK_RUNTIME_REPOSITORY_INDEXES
expect_failure repository_index_drift "$TMP/fixture"
MOCK_RUNTIME_REPOSITORY_INDEXES="$original_repository_indexes"
export MOCK_RUNTIME_REPOSITORY_INDEXES

original_runtime_lock="$MOCK_RUNTIME_PACKAGE_LOCK"
MOCK_RUNTIME_PACKAGE_LOCK="${MOCK_RUNTIME_PACKAGE_LOCK/termux-runtime-package-lock-v1.json/runtime-package-lock-v1.json}"
export MOCK_RUNTIME_PACKAGE_LOCK
expect_failure runtime_package_lock_basename_drift "$TMP/fixture"
grep -Fq 'ERROR: runtime_probe_invalid' \
  "$TMP/runtime_package_lock_basename_drift.log" \
  || fail runtime_package_lock_basename_drift_failed_for_wrong_reason
MOCK_RUNTIME_PACKAGE_LOCK="$original_runtime_lock"
export MOCK_RUNTIME_PACKAGE_LOCK

MOCK_RUNTIME_PACKAGE_LOCK="${MOCK_RUNTIME_PACKAGE_LOCK/termux-runtime-package-lock-v1.json\\t/termux-runtime-package-lock-v1.json\\t0}"
export MOCK_RUNTIME_PACKAGE_LOCK
expect_failure runtime_package_lock_drift "$TMP/fixture"
MOCK_RUNTIME_PACKAGE_LOCK="$original_runtime_lock"
export MOCK_RUNTIME_PACKAGE_LOCK

original_runtime_user="$MOCK_RUNTIME_USER"
MOCK_RUNTIME_USER=0:0
export MOCK_RUNTIME_USER
expect_failure runtime_user_inspect_drift "$TMP/fixture"
grep -Fq 'ERROR: runtime_image_inspect_invalid' \
  "$TMP/runtime_user_inspect_drift.log" \
  || fail runtime_user_inspect_drift_failed_for_wrong_reason
MOCK_RUNTIME_USER="$original_runtime_user"
export MOCK_RUNTIME_USER

original_uid_gid="$MOCK_RUNTIME_UID_GID"
MOCK_RUNTIME_UID_GID=1000:0
export MOCK_RUNTIME_UID_GID
expect_failure runtime_user_effective_drift "$TMP/fixture"
grep -Fq 'ERROR: runtime_probe_failed' \
  "$TMP/runtime_user_effective_drift.log" \
  || fail runtime_user_effective_drift_failed_for_wrong_reason
MOCK_RUNTIME_UID_GID="$original_uid_gid"
export MOCK_RUNTIME_UID_GID

original_linker="$MOCK_LINKER_SHA"
MOCK_LINKER_SHA="$(printf '0%.0s' {1..64})"
export MOCK_LINKER_SHA
expect_failure linker_drift "$TMP/fixture"
MOCK_LINKER_SHA="$original_linker"
export MOCK_LINKER_SHA

mkdir -m 700 "$TMP/conflict"
printf 'foreign\n' >"$TMP/conflict/termux-runtime-snapshot-replay-v1.json"
chmod 600 "$TMP/conflict/termux-runtime-snapshot-replay-v1.json"
if run_verifier \
  "$TMP/fixture" \
  "$TMP/conflict/termux-runtime-snapshot-replay-v1.json" \
  >"$TMP/conflict.log" 2>&1
then
  fail output_conflict_was_accepted
fi
[[ "$(<"$TMP/conflict/termux-runtime-snapshot-replay-v1.json")" == foreign ]] \
  || fail output_conflict_deleted_or_replaced_foreign_file

for forbidden in \
  'docker build' \
  'docker pull' \
  'apt-get install' \
  'apt-get update' \
  'pkg install' \
  'dpkg --unpack' \
  'dpkg --install' \
  'dpkg --configure'
do
  if grep -Fq "$forbidden" "$VERIFIER"; then
    fail "replay verifier contains construction/network command: $forbidden"
  fi
done
grep -Fq '"--network",' "$VERIFIER" || fail docker_network_argument_missing
grep -Fq '"none",' "$VERIFIER" || fail docker_network_none_missing
grep -Fq 'runtime_image_preexisting' "$VERIFIER" \
  || fail preexisting_image_rejection_missing
grep -Fq 'archive_structure_invalid' "$VERIFIER" \
  || fail archive_structure_rejection_missing
grep -Fq 'duplicate key' "$VERIFIER" \
  || fail duplicate_json_key_rejection_missing

printf 'Runtime snapshot replay contract passed\n'

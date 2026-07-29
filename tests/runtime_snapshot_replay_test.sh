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
    printf '[{"Id":"%s","Os":"linux","Architecture":"arm64","Config":{"User":"%s"},"RepoTags":["%s"],"RootFS":{"Type":"layers","Layers":["%s"]}}]\n' \
      "$MOCK_RUNTIME_ID" "$MOCK_RUNTIME_USER" "$MOCK_RUNTIME_TAG" "$MOCK_RUNTIME_LAYER"
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
layer_raw = b"fixture-layer"
layer = "sha256:" + hashlib.sha256(layer_raw).hexdigest()
config = json.dumps(
    {
        "architecture": "arm64",
        "config": {"User": "1000:1000"},
        "os": "linux",
        "rootfs": {"type": "layers", "diff_ids": [layer]},
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
        "Layers": ["runtime/layer.tar"],
    }
]
archive_path = root / "termux-qualified-runtime-image-v1.tar.gz"
with archive_path.open("wb") as raw:
    with gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as archive:
            for name, data in (
                (f"{runtime_id.removeprefix('sha256:')}.json", config),
                ("runtime/layer.tar", layer_raw),
                (
                    "manifest.json",
                    json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode(),
                ),
            ):
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
    "rootfsLayers": [layer],
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
            f"MOCK_RUNTIME_LAYER={layer}",
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
oci_root.mkdir(mode=0o700)
for name in (
    "termux-runtime-package-lock-v1.json",
    "termux-native-aggregate-evidence-v4.json",
    "automated-native-deployment-v1.json",
    "fixture-env",
):
    (oci_root / name).write_bytes((root / name).read_bytes())

layer_buffer = io.BytesIO()
with gzip.GzipFile(fileobj=layer_buffer, mode="wb", filename="", mtime=0) as layer_gzip:
    layer_gzip.write(layer_raw)
layer_blob = layer_buffer.getvalue()
layer_blob_digest = hashlib.sha256(layer_blob).hexdigest()
layer_name = f"blobs/sha256/{layer_blob_digest}"
config_name = f"blobs/sha256/{runtime_id.removeprefix('sha256:')}"
oci_manifest = {
    "schemaVersion": 2,
    "mediaType": "application/vnd.oci.image.manifest.v1+json",
    "config": {
        "mediaType": "application/vnd.oci.image.config.v1+json",
        "digest": runtime_id,
        "size": len(config),
        "platform": {"architecture": "arm64", "os": "linux"},
    },
    "layers": [
        {
            "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
            "digest": f"sha256:{layer_blob_digest}",
            "size": len(layer_blob),
        }
    ],
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
            "platform": {"architecture": "arm64", "os": "linux"},
        }
    ],
}
legacy_manifest = [
    {
        "Config": config_name,
        "RepoTags": [runtime_tag],
        "Layers": [layer_name],
    }
]
repositories = {
    "termux-mcp-qualified-runtime": {commit: layer_blob_digest}
}
oci_archive_path = oci_root / "termux-qualified-runtime-image-v1.tar.gz"
with oci_archive_path.open("wb") as raw:
    with gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as archive:
            for directory in ("blobs/", "blobs/sha256/"):
                info = tarfile.TarInfo(directory)
                info.type = tarfile.DIRTYPE
                info.mode = 0o755
                info.mtime = 0
                archive.addfile(info)
            for name, data in (
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
                        oci_index, sort_keys=True, separators=(",", ":")
                    ).encode(),
                ),
                (oci_manifest_name, oci_manifest_raw),
                (config_name, config),
                (layer_name, layer_blob),
                (
                    "manifest.json",
                    json.dumps(
                        legacy_manifest, sort_keys=True, separators=(",", ":")
                    ).encode(),
                ),
                (
                    "repositories",
                    json.dumps(
                        repositories, sort_keys=True, separators=(",", ":")
                    ).encode(),
                ),
            ):
                info = tarfile.TarInfo(name)
                info.size = len(data)
                info.mode = 0o644
                info.mtime = 0
                archive.addfile(info, io.BytesIO(data))
oci_archive_raw = oci_archive_path.read_bytes()
oci_snapshot = json.loads(json.dumps(snapshot))
oci_snapshot["archive"]["sha256"] = hashlib.sha256(oci_archive_raw).hexdigest()
oci_snapshot["archive"]["bytes"] = len(oci_archive_raw)
(oci_root / "termux-runtime-snapshot-v1.json").write_text(
    json.dumps(oci_snapshot, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
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

if operation == "unreferenced":
    entries.append(["unreferenced.bin", b"unreferenced", 0o644])
elif operation == "unsafe":
    entries.append(["../escape", b"escape", 0o644])
elif operation == "oci-platform":
    index = file_index("index.json")
    value = json.loads(entries[index][1])
    value["manifests"][0]["platform"]["architecture"] = "amd64"
    entries[index][1] = json.dumps(
        value, sort_keys=True, separators=(",", ":")
    ).encode()
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

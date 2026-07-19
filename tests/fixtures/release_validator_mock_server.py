#!/usr/bin/env python3
"""Deterministic HTTP fixture for termux_release_validate.sh shell tests."""

from __future__ import annotations

import base64
import ctypes
import hashlib
import hmac
import json
import os
import pathlib
import re
import stat
import struct
import sys
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any


def load_literal_runtime_config() -> tuple[bool, dict[str, str]]:
    raw_path = os.environ.get("MCP__CAPABILITY__CONFIG_FILE")
    if raw_path is None:
        return False, {}
    path = pathlib.Path(raw_path)
    if not path.is_absolute():
        raise SystemExit(2)
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NONBLOCK
    flags |= getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
        try:
            metadata = os.fstat(descriptor)
            if (
                not stat.S_ISREG(metadata.st_mode)
                or metadata.st_mode & 0o077
                or not metadata.st_mode & 0o400
                or metadata.st_size > 65_536
            ):
                raise SystemExit(2)
            chunks: list[bytes] = []
            total = 0
            while total < 65_537:
                chunk = os.read(descriptor, 65_537 - total)
                if not chunk:
                    break
                chunks.append(chunk)
                total += len(chunk)
            content = b"".join(chunks)
        finally:
            os.close(descriptor)
    except OSError as error:
        raise SystemExit(2) from error
    if len(content) > 65_536 or b"\r" in content or b"\0" in content:
        raise SystemExit(2)
    try:
        text = content.decode("utf-8")
    except UnicodeDecodeError as error:
        raise SystemExit(2) from error
    values: dict[str, str] = {}
    for line in text.splitlines():
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            raise SystemExit(2)
        name, value = line.split("=", 1)
        if (
            re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name) is None
            or not (name.startswith("MCP__") or name in {"RUST_LOG", "RUST_BACKTRACE"})
            or name in values
        ):
            raise SystemExit(2)
        values[name] = value
    return True, values


CONFIG_FILE_ACTIVE, RUNTIME_CONFIG = load_literal_runtime_config()


def runtime_value(name: str, default: str | None = None) -> str | None:
    if CONFIG_FILE_ACTIVE:
        return RUNTIME_CONFIG.get(name, default)
    return os.environ.get(name, default)


POSTURE = sys.argv[1]
VERSION = sys.argv[2] if len(sys.argv) > 2 else ""
MCP_ENABLED = POSTURE in {"mcp", "volume-control"}
WRITE_QUARANTINE_COMPONENT = ".termux-mcp-write-quarantine"
VOLUME_CONTROL_COMPILED = POSTURE == "volume-control"
PORT = int(runtime_value("MCP__SERVER__PORT", "0") or "0")
TOKEN = runtime_value("MCP__AUTH__STATIC_TOKEN")
SAFE_ROOT_VALUE = runtime_value("MCP__FILE__SAFE_ROOTS")
if TOKEN is None or SAFE_ROOT_VALUE is None:
    raise SystemExit(2)
SAFE_ROOT = pathlib.Path(SAFE_ROOT_VALUE).resolve()
MAX_BODY = int(runtime_value("MCP__TRANSPORT__MAX_BODY_BYTES", "1024") or "1024")
SSE_ENABLED = runtime_value("MCP__TRANSPORT__SSE_ENABLED", "false") == "true"
SESSION_ID = "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee"
CREATE_DIRECTORY_CAPABILITY_ENABLED = (
    runtime_value("MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED", "false") == "true"
)
WRITE_FILE_CAPABILITY_ENABLED = (
    runtime_value("MCP__FILE__WRITE_MUTATION_ENABLED", "false") == "true"
)
CAPABILITY_KEY_ID = runtime_value("MCP__CAPABILITY__KEY_ID", "") or ""
CAPABILITY_KEY_HEX = runtime_value("MCP__CAPABILITY__HMAC_KEY_HEX", "") or ""
CAPABILITY_HEADER = "MCP-Capability-Grant"
CONSUMED_GRANTS: set[bytes] = set()
TOOLS = [
    "runtime_status",
    "platform_info",
    "android_status",
    "project_service_status",
    "create_directory",
    "copy_file",
    "find_paths",
    "hash_file",
    "list_directory",
    "path_metadata",
    "read_binary_file",
    "read_binary_range",
    "read_file",
    "read_text_range",
    "search_text",
    "write_file",
]
MAX_WRITE_FILE_BYTES = 1_048_576
MAX_WRITE_FILE_RESPONSE_BYTES = 16_384


def payload_bytes(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def result(identifier: Any, structured: dict[str, Any]) -> dict[str, Any]:
    return {
        "jsonrpc": "2.0",
        "id": identifier,
        "result": {
            "content": [{"type": "text", "text": "fixture-result"}],
            "structuredContent": structured,
            "isError": False,
        },
    }


def rpc_error(identifier: Any, code: int, message: str, data: str) -> dict[str, Any]:
    return {
        "jsonrpc": "2.0",
        "id": identifier,
        "error": {"code": code, "message": message, "data": data},
    }


def capability_error(identifier: Any, reason: str) -> dict[str, Any]:
    return {
        "jsonrpc": "2.0",
        "id": identifier,
        "error": {
            "code": -32003,
            "message": "Capability authorization denied",
            "data": {"reason": reason},
        },
    }


def is_write_quarantine_component(component: str) -> bool:
    return component.isascii() and component.lower() == WRITE_QUARANTINE_COMPONENT


def safe_path(raw: str) -> pathlib.Path | None:
    try:
        candidate = pathlib.Path(raw)
        if candidate.is_symlink():
            return None
        resolved_parent = candidate.parent.resolve(strict=True)
        resolved = resolved_parent / candidate.name
        if os.path.commonpath((str(SAFE_ROOT), str(resolved))) != str(SAFE_ROOT):
            return None
        relative = resolved.relative_to(SAFE_ROOT)
        if any(is_write_quarantine_component(part) for part in relative.parts):
            return None
        return resolved
    except (OSError, ValueError):
        return None


def publish_fixture_write(
    target: pathlib.Path, content: bytes, disposition: str
) -> bool:
    """Emulate bounded native publication and return recovery retention state."""
    quarantine = target.parent / ".termux-mcp-write-quarantine"
    try:
        quarantine.mkdir(mode=0o700)
    except FileExistsError:
        pass
    quarantine_metadata = quarantine.stat(follow_symlinks=False)
    if (
        quarantine.is_symlink()
        or not stat.S_ISDIR(quarantine_metadata.st_mode)
        or stat.S_IMODE(quarantine_metadata.st_mode) != 0o700
    ):
        raise OSError("fixture quarantine is invalid")

    artifacts = list(quarantine.iterdir())
    if len(artifacts) >= 32:
        raise OSError("fixture quarantine artifact limit exceeded")
    total_bytes = 0
    for artifact in artifacts:
        metadata = artifact.stat(follow_symlinks=False)
        identifier = artifact.name.removeprefix(".termux-mcp-write-artifact-")
        try:
            canonical_identifier = str(uuid.UUID(identifier))
        except ValueError:
            canonical_identifier = ""
        if (
            artifact.is_symlink()
            or not stat.S_ISREG(metadata.st_mode)
            or canonical_identifier != identifier
            or metadata.st_nlink != 1
            or metadata.st_size > 1024 * 1024
        ):
            raise OSError("fixture quarantine entry is invalid")
        total_bytes += metadata.st_size
    if total_bytes + (target.stat(follow_symlinks=False).st_size if disposition == "replace" else 0) > 32 * 1024 * 1024:
        raise OSError("fixture quarantine byte limit exceeded")

    staging = quarantine / f".termux-mcp-write-artifact-{uuid.uuid4()}"
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC
    flags |= getattr(os, "O_NOFOLLOW", 0)
    descriptor: int | None = None
    retain_staging = False
    try:
        descriptor = os.open(staging, flags, 0o600)
        os.fchmod(descriptor, 0o600)
        view = memoryview(content)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise OSError("fixture staging write made no progress")
            view = view[written:]
        os.fsync(descriptor)
        staged = os.fstat(descriptor)
        if (
            not stat.S_ISREG(staged.st_mode)
            or stat.S_IMODE(staged.st_mode) != 0o600
            or staged.st_size != len(content)
        ):
            raise OSError("fixture staging verification failed")
        staged_descriptor = descriptor
        descriptor = None
        os.close(staged_descriptor)

        if disposition == "create":
            os.link(staging, target, follow_symlinks=False)
            staging.unlink()
        elif disposition == "replace":
            libc = ctypes.CDLL(None, use_errno=True)
            renameat2 = getattr(libc, "renameat2", None)
            if renameat2 is None:
                raise OSError("fixture requires atomic rename exchange support")
            renameat2.argtypes = [
                ctypes.c_int,
                ctypes.c_char_p,
                ctypes.c_int,
                ctypes.c_char_p,
                ctypes.c_uint,
            ]
            renameat2.restype = ctypes.c_int
            if renameat2(
                -100,
                os.fsencode(staging),
                -100,
                os.fsencode(target),
                2,
            ) != 0:
                error = ctypes.get_errno()
                raise OSError(error, os.strerror(error))
            # EXCHANGE leaves the displaced prior object at the randomized
            # staging name. Match the native non-destructive recovery contract,
            # including after any later verification or durability failure.
            retain_staging = True
        else:
            raise ValueError("unsupported write disposition")

        published = target.stat(follow_symlinks=False)
        if (
            not stat.S_ISREG(published.st_mode)
            or stat.S_IMODE(published.st_mode) != 0o600
            or published.st_size != len(content)
            or (published.st_dev, published.st_ino) != (staged.st_dev, staged.st_ino)
        ):
            raise OSError("fixture publication verification failed")
        parent_descriptor = os.open(
            target.parent,
            os.O_RDONLY | os.O_CLOEXEC | getattr(os, "O_DIRECTORY", 0),
        )
        try:
            os.fsync(parent_descriptor)
        finally:
            os.close(parent_descriptor)
        return disposition == "replace"
    finally:
        if descriptor is not None:
            os.close(descriptor)
        if not retain_staging:
            try:
                staging.unlink()
            except FileNotFoundError:
                pass


def grant_binding(
    purpose: str,
    session_id: str,
    target: pathlib.Path,
    content: bytes | None = None,
    disposition: str | None = None,
) -> bytes:
    relative = target.relative_to(SAFE_ROOT)
    root_stat = SAFE_ROOT.stat()
    key = bytes.fromhex(CAPABILITY_KEY_HEX)
    principal = hmac.new(
        key,
        b"termux-mcp:static-principal:v1\0" + TOKEN.encode(),
        hashlib.sha256,
    ).digest()
    digest = hashlib.sha256()
    digest.update(b"termux-mcp-release-fixture:request-capability:v2\0")
    digest.update(purpose.encode())
    digest.update(b"\0")
    for value in (principal, session_id.encode()):
        digest.update(struct.pack(">I", len(value)))
        digest.update(value)
    digest.update(struct.pack(">QQ", root_stat.st_dev, root_stat.st_ino))
    for component in relative.parts:
        encoded = os.fsencode(component)
        digest.update(struct.pack(">I", len(encoded)))
        digest.update(encoded)
    if purpose == "create_directory":
        digest.update(b"filesystem.create-directory\0mutating")
    elif purpose == "write_file" and content is not None and disposition is not None:
        digest.update(b"filesystem.write-file\0mutating\0")
        digest.update(disposition.encode())
        digest.update(b"\0")
        digest.update(hashlib.sha256(content).digest())
        if disposition == "replace":
            metadata = target.stat(follow_symlinks=False)
            digest.update(struct.pack(">QQ", metadata.st_dev, metadata.st_ino))
        else:
            digest.update(bytes(16))
    else:
        raise ValueError("unsupported fixture grant purpose")
    return digest.digest()


def write_grant_binding(
    grant_id: bytes,
    session_id: str,
    target: pathlib.Path,
    content: bytes,
    disposition: str,
) -> bytes:
    key = bytes.fromhex(CAPABILITY_KEY_HEX)
    principal = hmac.new(
        key,
        b"termux-mcp:write-file-principal:v1\0" + TOKEN.encode(),
        hashlib.sha256,
    ).digest()
    try:
        parsed_session = uuid.UUID(session_id)
    except ValueError as error:
        raise ValueError("invalid canonical session") from error
    if str(parsed_session) != session_id:
        raise ValueError("invalid canonical session")

    relative = target.relative_to(SAFE_ROOT)
    target_digest = hashlib.sha256()
    target_digest.update(b"termux-mcp:write-file-target:v1\0")
    component_count = 0
    for component in relative.parts:
        encoded = os.fsencode(component)
        target_digest.update(struct.pack(">I", len(encoded)))
        target_digest.update(encoded)
        component_count += 1
    if component_count == 0:
        raise ValueError("empty target")
    target_digest.update(struct.pack(">I", component_count))

    if disposition == "create":
        disposition_code = 1
        replacement = bytes([0]) + bytes(56)
    elif disposition == "replace":
        disposition_code = 2
        metadata = target.stat(follow_symlinks=False)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_nlink != 1
            or metadata.st_size > 1_048_576
        ):
            raise ValueError("invalid replacement identity")
        ctime_seconds, ctime_nanoseconds = divmod(metadata.st_ctime_ns, 1_000_000_000)
        replacement = (
            bytes([1])
            + struct.pack(
                ">QQQqqQ",
                metadata.st_dev,
                metadata.st_ino,
                metadata.st_size,
                ctime_seconds,
                ctime_nanoseconds,
                metadata.st_nlink,
            )
            + bytes(8)
        )
    else:
        raise ValueError("invalid write disposition")

    root_stat = SAFE_ROOT.stat()
    operation = (
        b"termux-mcp:write-file-operation-binding:v1\0"
        + grant_id
        + principal
        + parsed_session.bytes
        + bytes([3, 1])
        + struct.pack(">QQ", root_stat.st_dev, root_stat.st_ino)
        + target_digest.digest()
        + hashlib.sha256(content).digest()
        + bytes([disposition_code])
        + replacement
    )
    return hmac.new(key, operation, hashlib.sha256).digest()


def read_private_write_content() -> tuple[bytes, os.stat_result]:
    raw_path = os.environ.get("MCP__CAPABILITY__WRITE_FILE_CONTENT_FILE", "")
    path = pathlib.Path(raw_path)
    if not path.is_absolute():
        raise SystemExit(2)
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NONBLOCK
    flags |= getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
        try:
            before = os.fstat(descriptor)
            if (
                not stat.S_ISREG(before.st_mode)
                or before.st_mode & 0o077
                or not before.st_mode & 0o400
                or before.st_size > 1_048_576
            ):
                raise SystemExit(2)
            content = b""
            while len(content) <= 1_048_576:
                chunk = os.read(descriptor, 1_048_577 - len(content))
                if not chunk:
                    break
                content += chunk
            after = os.fstat(descriptor)
        finally:
            os.close(descriptor)
    except OSError as error:
        raise SystemExit(2) from error
    if (
        len(content) > 1_048_576
        or (
            before.st_dev,
            before.st_ino,
            before.st_size,
            before.st_mode,
            before.st_ctime_ns,
            before.st_mtime_ns,
        )
        != (
            after.st_dev,
            after.st_ino,
            after.st_size,
            after.st_mode,
            after.st_ctime_ns,
            after.st_mtime_ns,
        )
    ):
        raise SystemExit(2)
    try:
        content.decode("utf-8")
    except UnicodeDecodeError as error:
        raise SystemExit(2) from error
    return content, before


def issue_fixture_grant(purpose: str) -> str:
    enabled = (
        CREATE_DIRECTORY_CAPABILITY_ENABLED
        if purpose == "create_directory"
        else WRITE_FILE_CAPABILITY_ENABLED
    )
    if not enabled:
        raise SystemExit(2)
    try:
        key = bytes.fromhex(CAPABILITY_KEY_HEX)
    except ValueError as error:
        raise SystemExit(2) from error
    if len(key) != 32 or not CAPABILITY_KEY_ID:
        raise SystemExit(2)
    session_id = os.environ.get("MCP__CAPABILITY__SESSION_ID", "")
    if purpose == "create_directory":
        target = safe_path(os.environ.get("MCP__CAPABILITY__CREATE_DIRECTORY_TARGET", ""))
        content = None
        disposition = None
        payload_size = 130
        content_identity = None
    elif purpose == "write_file":
        target = safe_path(os.environ.get("MCP__CAPABILITY__WRITE_FILE_TARGET", ""))
        content, content_identity = read_private_write_content()
        disposition = os.environ.get("MCP__CAPABILITY__WRITE_FILE_DISPOSITION", "")
        payload_size = 64
    else:
        raise SystemExit(2)
    if not session_id or target is None or target == SAFE_ROOT or not target.parent.is_dir():
        raise SystemExit(2)
    if purpose == "create_directory" and (target.exists() or target.is_symlink()):
        raise SystemExit(2)
    if purpose == "write_file":
        if disposition == "create" and (target.exists() or target.is_symlink()):
            raise SystemExit(2)
        if disposition == "replace" and (
            target.is_symlink() or not target.is_file()
        ):
            raise SystemExit(2)
        if disposition not in {"create", "replace"}:
            raise SystemExit(2)
        if disposition == "replace":
            target_metadata = target.stat(follow_symlinks=False)
            if (target_metadata.st_dev, target_metadata.st_ino) == (
                content_identity.st_dev,
                content_identity.st_ino,
            ):
                raise SystemExit(2)
        raw_config = os.environ.get("MCP__CAPABILITY__CONFIG_FILE")
        if raw_config:
            try:
                config_metadata = pathlib.Path(raw_config).stat(follow_symlinks=False)
            except OSError as error:
                raise SystemExit(2) from error
            if (config_metadata.st_dev, config_metadata.st_ino) == (
                content_identity.st_dev,
                content_identity.st_ino,
            ):
                raise SystemExit(2)
    issued = int(time.time())
    grant_id = os.urandom(16)
    if purpose == "write_file":
        binding = write_grant_binding(
            grant_id, session_id, target, content, disposition
        )
    else:
        binding = grant_binding(purpose, session_id, target, content, disposition)
    payload = grant_id + binding + struct.pack(">QQ", issued, issued + 60)
    payload += bytes(payload_size - len(payload))
    if len(payload) != payload_size:
        raise SystemExit(2)
    signed = f"v1.{CAPABILITY_KEY_ID}.{payload.hex()}"
    signature = hmac.new(key, signed.encode(), hashlib.sha256).hexdigest()
    return f"{signed}.{signature}"


def consume_fixture_grant(
    raw: str | None,
    purpose: str,
    session_id: str,
    target: pathlib.Path,
    content: bytes | None = None,
    disposition: str | None = None,
) -> str | None:
    if raw is None:
        return "capability_grant_missing"
    parts = raw.split(".")
    if len(parts) != 4:
        return "capability_grant_malformed"
    version, key_id, payload_hex, signature_hex = parts
    if version != "v1":
        return "capability_grant_version_unknown"
    if key_id != CAPABILITY_KEY_ID:
        return "capability_grant_key_unknown"
    try:
        payload = bytes.fromhex(payload_hex)
        signature = bytes.fromhex(signature_hex)
        key = bytes.fromhex(CAPABILITY_KEY_HEX)
    except ValueError:
        return "capability_grant_malformed"
    expected_payload_size = 130 if purpose == "create_directory" else 64
    if len(payload) != expected_payload_size or len(signature) != 32 or len(key) != 32:
        return "capability_grant_malformed"
    signed = f"{version}.{key_id}.{payload_hex}".encode()
    expected = hmac.new(key, signed, hashlib.sha256).digest()
    if not hmac.compare_digest(signature, expected):
        return "capability_grant_signature_invalid"
    grant_id = payload[:16]
    binding = payload[16:48]
    issued, expires = struct.unpack(">QQ", payload[48:64])
    current = int(time.time())
    try:
        expected_binding = (
            write_grant_binding(grant_id, session_id, target, content, disposition)
            if purpose == "write_file" and content is not None and disposition is not None
            else grant_binding(purpose, session_id, target, content, disposition)
        )
    except (OSError, ValueError):
        return "capability_grant_binding_mismatch"
    if not hmac.compare_digest(binding, expected_binding):
        return "capability_grant_binding_mismatch"
    if issued > current + 5:
        return "capability_grant_future_issued"
    if expires <= issued or expires - issued > 120:
        return "capability_grant_lifetime_exceeded"
    if current >= expires:
        return "capability_grant_expired"
    if grant_id in CONSUMED_GRANTS:
        return "capability_grant_replayed"
    CONSUMED_GRANTS.add(grant_id)
    return None


class Handler(BaseHTTPRequestHandler):
    server_version = "termux-mcp-release-fixture"
    sys_version = ""

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def send_bytes(
        self,
        status: int,
        body: bytes = b"",
        headers: dict[str, str] | None = None,
        content_type: str = "application/json",
    ) -> None:
        self.send_response(status)
        if body:
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        self.end_headers()
        if body:
            self.wfile.write(body)

    def send_json(self, status: int, value: Any, headers: dict[str, str] | None = None) -> None:
        self.send_bytes(status, payload_bytes(value), headers)

    def authenticated(self) -> bool:
        if self.headers.get("Authorization") == f"Bearer {TOKEN}":
            return True
        self.send_json(
            401,
            {"error": "unauthorized", "message": "Bearer authentication required."},
            {"WWW-Authenticate": "Bearer", "Cache-Control": "no-store"},
        )
        return False

    def active_session(self) -> bool:
        if not self.headers.get("MCP-Protocol-Version"):
            self.send_json(
                400,
                {
                    "error": "protocol_version_required",
                    "message": "MCP-Protocol-Version is required after initialization.",
                },
            )
            return False
        if self.headers.get("MCP-Session-Id") != SESSION_ID:
            self.send_json(
                404,
                {"error": "session_not_found", "message": "Session not found."},
            )
            return False
        return True

    def transport_allowed(self) -> bool:
        allowed_hosts = {f"localhost:{PORT}", f"127.0.0.1:{PORT}"}
        allowed_origins = {f"http://localhost:{PORT}", f"http://127.0.0.1:{PORT}"}
        if self.headers.get("Host") not in allowed_hosts:
            self.send_json(
                403,
                {"error": "transport_security_rejected", "message": "host_not_allowed"},
            )
            return False
        origin = self.headers.get("Origin")
        if origin is None:
            self.send_json(
                403,
                {"error": "transport_security_rejected", "message": "origin_required"},
            )
            return False
        if origin not in allowed_origins:
            self.send_json(
                403,
                {"error": "transport_security_rejected", "message": "origin_not_allowed"},
            )
            return False
        return True

    def do_GET(self) -> None:
        if self.path == "/health":
            self.send_bytes(200, b"ok", content_type="text/plain")
            return
        if self.path == "/ready":
            ready: dict[str, Any] = {
                "status": "ready",
                "version": VERSION,
                "mcp_runtime_enabled": MCP_ENABLED,
                "safe_root_count": 1,
                "auth_posture": "static_token",
            }
            if MCP_ENABLED:
                ready["mcp_request_limits"] = {
                    "max_concurrent_requests": 4,
                    "request_timeout_seconds": 30,
                    "max_body_bytes": MAX_BODY,
                    "sse_enabled": SSE_ENABLED,
                }
            self.send_json(200, ready)
            return
        if self.path != "/mcp" or not MCP_ENABLED:
            self.send_json(404, {"error": "not_found"})
            return
        if not self.authenticated() or not self.transport_allowed() or not self.active_session():
            return
        self.send_bytes(405)

    def do_DELETE(self) -> None:
        if self.path != "/mcp" or not MCP_ENABLED:
            self.send_json(404, {"error": "not_found"})
            return
        if not self.authenticated() or not self.transport_allowed() or not self.active_session():
            return
        self.send_bytes(204)

    def do_POST(self) -> None:
        if self.path != "/mcp" or not MCP_ENABLED:
            self.send_json(404, {"error": "not_found"})
            return
        if not self.authenticated() or not self.transport_allowed():
            return
        length = int(self.headers.get("Content-Length", "0"))
        if length > MAX_BODY:
            self.rfile.read(length)
            self.send_json(
                413,
                {
                    "error": "mcp_request_body_too_large",
                    "message": "Request body too large.",
                },
            )
            return
        try:
            request = json.loads(self.rfile.read(length))
        except (json.JSONDecodeError, UnicodeDecodeError):
            self.send_json(400, rpc_error(None, -32700, "Parse error", "Invalid JSON."))
            return

        method = request.get("method")
        identifier = request.get("id")
        grant_values = self.headers.get_all(CAPABILITY_HEADER) or []
        if len(grant_values) > 1 or any(
            not value.isascii() or not value or len(value) > 512
            for value in grant_values
        ):
            self.send_json(
                400,
                {
                    "error": "invalid_capability_grant_header",
                    "message": "Capability grant header is invalid.",
                },
            )
            return
        grant = grant_values[0] if grant_values else None
        if method == "initialize":
            if grant is not None:
                self.send_json(
                    400,
                    rpc_error(
                        identifier,
                        -32600,
                        "Invalid Request",
                        "Capability context is not allowed.",
                    ),
                )
                return
            self.send_json(
                200,
                {
                    "jsonrpc": "2.0",
                    "id": identifier,
                    "result": {
                        "protocolVersion": "2025-11-25",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "termux-mcp-edge", "version": VERSION},
                    },
                },
                {"MCP-Session-Id": SESSION_ID},
            )
            return
        if not self.active_session():
            return
        if method == "notifications/initialized":
            if grant is not None:
                self.send_json(
                    400,
                    rpc_error(None, -32600, "Invalid Request", "Capability context is not allowed."),
                )
                return
            self.send_bytes(202)
            return
        if method == "tools/list":
            if grant is not None:
                self.send_json(
                    400,
                    rpc_error(
                        identifier,
                        -32600,
                        "Invalid Request",
                        "Capability context is not allowed.",
                    ),
                )
                return
            tools = []
            for name in TOOLS:
                if name == "create_directory":
                    create_dry_run: dict[str, Any] = {"type": "boolean"}
                    if not CREATE_DIRECTORY_CAPABILITY_ENABLED:
                        create_dry_run["const"] = True
                    tools.append(
                        {
                            "name": name,
                            "description": (
                                "Fixture create_directory requires MCP-Capability-Grant for explicit mutation."
                                if CREATE_DIRECTORY_CAPABILITY_ENABLED
                                else "Fixture create_directory mutation gate is disabled."
                            ),
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": {"type": "string"},
                                    "dry_run": create_dry_run,
                                },
                                "required": ["path"],
                                "additionalProperties": False,
                            },
                        }
                    )
                elif name == "find_paths":
                    tools.append(
                        {
                            "name": name,
                            "description": "Fixture bounded literal basename discovery.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": {"type": "string"},
                                    "query": {
                                        "type": "string",
                                        "minLength": 1,
                                        "maxLength": 256,
                                        "x-maxBytes": 256,
                                    },
                                    "kind": {
                                        "type": "string",
                                        "enum": ["any", "regular_file", "directory"],
                                    },
                                    "max_depth": {
                                        "type": "integer",
                                        "minimum": 1,
                                        "maximum": 5,
                                    },
                                },
                                "required": ["path", "query"],
                                "additionalProperties": False,
                            },
                        }
                    )
                elif name == "hash_file":
                    tools.append(
                        {
                            "name": name,
                            "description": "Fixture bounded safe-root SHA-256 file hashing.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {"path": {"type": "string"}},
                                "required": ["path"],
                                "additionalProperties": False,
                            },
                        }
                    )
                elif name == "read_binary_file":
                    tools.append(
                        {
                            "name": name,
                            "description": "Fixture bounded base64 safe-root file read.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {"path": {"type": "string"}},
                                "required": ["path"],
                                "additionalProperties": False,
                            },
                        }
                    )
                elif name == "read_binary_range":
                    tools.append(
                        {
                            "name": name,
                            "description": "Fixture bounded base64 safe-root file range read.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": {"type": "string"},
                                    "offset_bytes": {
                                        "type": "integer",
                                        "minimum": 0,
                                        "maximum": 67108864,
                                    },
                                    "length_bytes": {
                                        "type": "integer",
                                        "minimum": 1,
                                        "maximum": 262144,
                                    },
                                },
                                "required": ["path", "offset_bytes", "length_bytes"],
                                "additionalProperties": False,
                            },
                        }
                    )
                elif name == "read_text_range":
                    tools.append(
                        {
                            "name": name,
                            "description": "Fixture bounded UTF-8 safe-root file range read.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": {"type": "string"},
                                    "offset_bytes": {
                                        "type": "integer",
                                        "minimum": 0,
                                        "maximum": 67108864,
                                    },
                                    "max_bytes": {
                                        "type": "integer",
                                        "minimum": 4,
                                        "maximum": 262144,
                                    },
                                },
                                "required": ["path", "offset_bytes", "max_bytes"],
                                "additionalProperties": False,
                            },
                        }
                    )
                elif name == "write_file":
                    write_dry_run: dict[str, Any] = {"type": "boolean"}
                    if not WRITE_FILE_CAPABILITY_ENABLED:
                        write_dry_run["const"] = True
                    tools.append(
                        {
                            "name": name,
                            "description": (
                                "Fixture write_file requires a target/content/disposition-bound MCP-Capability-Grant for explicit mutation."
                                if WRITE_FILE_CAPABILITY_ENABLED
                                else "Fixture write_file mutation gate is disabled."
                            ),
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": {"type": "string"},
                                    "content": {
                                        "type": "string",
                                        "maxLength": 1_048_576,
                                        "x-maxBytes": 1_048_576,
                                    },
                                    "dry_run": write_dry_run,
                                },
                                "required": ["path", "content"],
                                "additionalProperties": False,
                            },
                        }
                    )
                else:
                    tools.append(
                        {
                            "name": name,
                            "description": "fixture",
                            "inputSchema": {"type": "object"},
                        }
                    )
            self.send_json(
                200,
                {
                    "jsonrpc": "2.0",
                    "id": identifier,
                    "result": {
                        "tools": tools
                    },
                },
            )
            return
        if method != "tools/call":
            self.send_json(501, rpc_error(identifier, -32601, "Method not found", "Unavailable."))
            return

        params = request.get("params") or {}
        name = params.get("name")
        arguments = params.get("arguments") or {}
        if grant is not None and name not in {"create_directory", "write_file"}:
            self.send_json(
                400,
                rpc_error(
                    identifier,
                    -32600,
                    "Invalid Request",
                    "Capability context is not allowed.",
                ),
            )
            return
        if name == "runtime_status":
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "commandExecution": False,
                        "androidPlatformTools": False,
                        "highImpactTools": False,
                        "serverSentEvents": SSE_ENABLED,
                        "serverSentEventsMode": (
                            "finite_request_response_with_origin_stream_replay"
                            if SSE_ENABLED
                            else "disabled"
                        ),
                        "sseMaxStreamsPerSession": 8,
                        "sseMaxEventsPerStream": 2,
                        "sseMaxEventDataBytes": 131072,
                        "sseMaxReplayBytesPerSession": 262144,
                        "sseMaxLastEventIdBytes": 64,
                        "sseRetryMilliseconds": 1000,
                        "androidVolumeControlCompiled": VOLUME_CONTROL_COMPILED,
                        "androidVolumeControlEnabled": False,
                        "androidVolumeGrantRequired": False,
                        "pathDiscovery": True,
                        "pathDiscoveryMatchMode": "case_sensitive_literal_basename",
                        "pathDiscoveryMaxDepth": 5,
                        "pathDiscoveryMaxEntries": 8192,
                        "pathDiscoveryMaxMatches": 512,
                        "pathDiscoveryMaxQueryBytes": 256,
                        "pathDiscoveryMaxResponseBytes": 262144,
                        "binaryFileReads": True,
                        "binaryFileReadEncoding": "base64",
                        "binaryFileReadMaxBytes": 1048576,
                        "binaryFileReadMaxResponseBytes": 1507328,
                        "binaryRangeReads": True,
                        "binaryRangeReadEncoding": "base64",
                        "binaryRangeReadMaxFileBytes": 67108864,
                        "binaryRangeReadMaxBytes": 262144,
                        "binaryRangeReadMaxResponseBytes": 393216,
                        "textRangeReads": True,
                        "textRangeReadEncoding": "utf-8",
                        "textRangeReadMinBytes": 4,
                        "textRangeReadMaxFileBytes": 67108864,
                        "textRangeReadMaxBytes": 262144,
                        "textRangeReadMaxResponseBytes": 1703936,
                        "fileHashing": True,
                        "fileHashAlgorithm": "sha256",
                        "fileHashMaxBytes": 16777216,
                        "createDirectoryMutationEnabled": CREATE_DIRECTORY_CAPABILITY_ENABLED,
                        "createDirectoryGrantRequired": CREATE_DIRECTORY_CAPABILITY_ENABLED,
                        "createDirectoryGrantHeader": "mcp-capability-grant",
                        "createDirectoryGrantTtlSeconds": 60,
                        "createDirectoryMutationMode": (
                            "dry_run_or_request_scoped_single_use_grant"
                            if CREATE_DIRECTORY_CAPABILITY_ENABLED
                            else "dry_run_only_mutation_disabled"
                        ),
                        "fileWrites": True,
                        "fileWriteMode": (
                            "dry_run_or_target_content_disposition_scoped_single_use_grant"
                            if WRITE_FILE_CAPABILITY_ENABLED
                            else "dry_run_only_mutation_disabled"
                        ),
                        "fileWriteMutationEnabled": WRITE_FILE_CAPABILITY_ENABLED,
                        "fileWriteGrantRequired": WRITE_FILE_CAPABILITY_ENABLED,
                        "fileWriteGrantHeader": "mcp-capability-grant",
                        "fileWriteGrantTtlSeconds": 60,
                        "fileWriteMaxBytes": 1048576,
                        "fileWriteMaxResponseBytes": 16384,
                    },
                ),
            )
            return
        if name == "set_android_volume" and VOLUME_CONTROL_COMPILED:
            response = result(
                identifier,
                {
                    "reasonCode": "volume_control_runtime_disabled",
                    "outcome": "denied",
                },
            )
            response["result"]["isError"] = True
            self.send_json(200, response)
            return
        if name == "platform_info":
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "os": "android",
                        "arch": "aarch64",
                        "family": "unix",
                        "available_parallelism": 8,
                        "package_version": VERSION,
                    },
                ),
            )
            return
        if name == "android_status":
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "status_mode": "read_only_allowlisted_status",
                        "target_os": "android",
                        "target_arch": "aarch64",
                        "target_family": "unix",
                        "package_version": VERSION,
                        "termux_runtime_hint": "android_termux_candidate",
                        "android_api_access": "not_used",
                        "android_control_enabled": False,
                        "shell_fallback_enabled": False,
                        "command_execution_enabled": False,
                        "high_impact_controls_enabled": False,
                    },
                ),
            )
            return
        if name == "project_service_status":
            if arguments.get("service_name") != "mcp_runtime":
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Tool arguments are invalid."),
                )
                return
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "service_name": "mcp_runtime",
                        "ownership": "project_owned_allowlisted",
                        "status_mode": "read_only_project_service_status",
                        "lifecycle_state": "available_in_process",
                        "health": "transport_runtime_available",
                        "pid_inspection_enabled": False,
                        "process_listing_enabled": False,
                        "command_line_exposed": False,
                        "environment_exposed": False,
                        "command_execution_enabled": False,
                        "mutation_enabled": False,
                    },
                ),
            )
            return
        if name == "create_directory":
            target = safe_path(str(arguments.get("path", "")))
            dry_run = arguments.get("dry_run", True)
            if (
                target is None
                or not isinstance(dry_run, bool)
                or target == SAFE_ROOT
                or not target.parent.is_dir()
                or target.exists()
                or target.is_symlink()
            ):
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Directory destination invalid."),
                )
                return
            if not dry_run:
                if not CREATE_DIRECTORY_CAPABILITY_ENABLED:
                    self.send_json(
                        403,
                        capability_error(identifier, "create_directory_mutation_disabled"),
                    )
                    return
                denial = consume_fixture_grant(
                    grant, "create_directory", SESSION_ID, target
                )
                if denial is not None:
                    self.send_json(403, capability_error(identifier, denial))
                    return
                target.mkdir(mode=0o700, parents=False, exist_ok=False)
                target.chmod(0o700)
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "path": str(target),
                        "dryRun": dry_run,
                        "mode": "0700",
                        "maxResponseBytes": 16384,
                    },
                ),
            )
            return
        if name == "copy_file":
            source = safe_path(str(arguments.get("source_path", "")))
            destination = safe_path(str(arguments.get("destination_path", "")))
            dry_run = arguments.get("dry_run", True)
            if (
                source is None
                or destination is None
                or not isinstance(dry_run, bool)
                or source == destination
                or source.is_symlink()
                or not source.is_file()
                or not destination.parent.is_dir()
                or destination.exists()
                or destination.is_symlink()
            ):
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "File copy invalid."),
                )
                return
            content = source.read_bytes()
            if len(content) > 1048576:
                self.send_json(
                    413,
                    rpc_error(identifier, -32001, "Payload too large", "File copy too large."),
                )
                return
            if not dry_run:
                descriptor = os.open(destination, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
                with os.fdopen(descriptor, "wb") as stream:
                    stream.write(content)
                    stream.flush()
                    os.fsync(stream.fileno())
                destination.chmod(0o600)
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "sourcePath": str(source),
                        "destinationPath": str(destination),
                        "dryRun": dry_run,
                        "sizeBytes": len(content),
                        "mode": "0600",
                        "maxFileBytes": 1048576,
                        "maxResponseBytes": 16384,
                    },
                ),
            )
            return
        if name == "find_paths":
            target = safe_path(str(arguments.get("path", "")))
            query = arguments.get("query")
            kind_filter = arguments.get("kind", "any")
            max_depth = arguments.get("max_depth", 5)
            if (
                target is None
                or not target.is_dir()
                or not isinstance(query, str)
                or not query
                or len(query.encode("utf-8")) > 256
                or any(character in query for character in ("\0", "\n", "\r", "/"))
                or kind_filter not in {"any", "regular_file", "directory"}
                or isinstance(max_depth, bool)
                or not isinstance(max_depth, int)
                or max_depth < 1
                or max_depth > 5
            ):
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Path discovery invalid."),
                )
                return
            matches: list[dict[str, str]] = []
            entries_examined = 0
            skipped_invalid_utf8_entries = 0
            skipped_unsafe_entries = 0
            skipped_unreadable_entries = 0
            truncated = False
            queue: list[tuple[pathlib.Path, int]] = [(target, 1)]
            while queue and not truncated:
                directory, depth = queue.pop(0)
                try:
                    entries = sorted(os.scandir(directory), key=lambda entry: entry.name)
                except OSError:
                    skipped_unreadable_entries += 1
                    continue
                for entry in entries:
                    if is_write_quarantine_component(entry.name):
                        continue
                    if entries_examined >= 8192 or len(matches) >= 512:
                        truncated = True
                        break
                    entries_examined += 1
                    try:
                        entry.name.encode("utf-8")
                    except UnicodeEncodeError:
                        skipped_invalid_utf8_entries += 1
                        continue
                    try:
                        if entry.is_symlink():
                            skipped_unsafe_entries += 1
                            continue
                        if entry.is_file(follow_symlinks=False):
                            entry_kind = "regular_file"
                        elif entry.is_dir(follow_symlinks=False):
                            entry_kind = "directory"
                        else:
                            skipped_unsafe_entries += 1
                            continue
                    except OSError:
                        skipped_unreadable_entries += 1
                        continue
                    entry_path = pathlib.Path(entry.path)
                    if query in entry.name and kind_filter in {"any", entry_kind}:
                        matches.append({"path": str(entry_path), "kind": entry_kind})
                    if entry_kind == "directory" and depth < max_depth:
                        queue.append((entry_path, depth + 1))
            matches.sort(key=lambda match: match["path"])
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "path": str(target),
                        "matches": matches,
                        "truncated": truncated,
                        "entriesExamined": entries_examined,
                        "skippedInvalidUtf8Entries": skipped_invalid_utf8_entries,
                        "skippedUnsafeEntries": skipped_unsafe_entries,
                        "skippedUnreadableEntries": skipped_unreadable_entries,
                        "queryBytes": len(query.encode("utf-8")),
                        "kindFilter": kind_filter,
                        "maxDepth": max_depth,
                        "maxEntries": 8192,
                        "maxMatches": 512,
                        "maxResponseBytes": 262144,
                    },
                ),
            )
            return
        if name == "hash_file":
            target = safe_path(str(arguments.get("path", "")))
            if target is None or target.is_symlink() or not target.is_file():
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "File hash invalid."),
                )
                return
            try:
                descriptor = os.open(
                    target,
                    os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                )
                with os.fdopen(descriptor, "rb") as stream:
                    metadata = os.fstat(stream.fileno())
                    if not stat.S_ISREG(metadata.st_mode):
                        self.send_json(
                            400,
                            rpc_error(
                                identifier,
                                -32602,
                                "Invalid params",
                                "File hash invalid.",
                            ),
                        )
                        return
                    if metadata.st_size > 16777216:
                        self.send_json(
                            413,
                            rpc_error(
                                identifier,
                                -32001,
                                "Payload too large",
                                "File hash too large.",
                            ),
                        )
                        return
                    content = stream.read(16777217)
            except OSError:
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "File hash invalid."),
                )
                return
            if len(content) > 16777216:
                self.send_json(
                    413,
                    rpc_error(identifier, -32001, "Payload too large", "File hash too large."),
                )
                return
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "algorithm": "sha256",
                        "digest": hashlib.sha256(content).hexdigest(),
                        "sizeBytes": len(content),
                    },
                ),
            )
            return
        if name == "list_directory":
            target = safe_path(str(arguments.get("path", "")))
            if target is None or not target.is_dir():
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Safe-root rejection."),
                )
                return
            entries = []
            for child in sorted(target.iterdir(), key=lambda item: str(item)):
                if is_write_quarantine_component(child.name):
                    continue
                metadata = child.stat()
                entries.append(
                    {
                        "path": str(child),
                        "size": metadata.st_size,
                        "is_dir": child.is_dir(),
                        "modified": None,
                    }
                )
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "path": str(target),
                        "entries": entries,
                        "truncated": False,
                        "maxEntries": 4096,
                        "maxResponseBytes": 262144,
                    },
                ),
            )
            return
        if name == "read_binary_file":
            target = safe_path(str(arguments.get("path", "")))
            if target is None or target.is_symlink() or not target.is_file():
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Binary read invalid."),
                )
                return
            try:
                descriptor = os.open(
                    target,
                    os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                )
                with os.fdopen(descriptor, "rb") as stream:
                    metadata = os.fstat(stream.fileno())
                    if not stat.S_ISREG(metadata.st_mode):
                        self.send_json(
                            400,
                            rpc_error(
                                identifier,
                                -32602,
                                "Invalid params",
                                "Binary read invalid.",
                            ),
                        )
                        return
                    if metadata.st_size > 1048576:
                        self.send_json(
                            413,
                            rpc_error(
                                identifier,
                                -32001,
                                "Payload too large",
                                "Binary read too large.",
                            ),
                        )
                        return
                    content = stream.read(1048577)
            except OSError:
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Binary read invalid."),
                )
                return
            if len(content) > 1048576:
                self.send_json(
                    413,
                    rpc_error(identifier, -32001, "Payload too large", "Binary read too large."),
                )
                return
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "encoding": "base64",
                        "data": base64.b64encode(content).decode("ascii"),
                        "sizeBytes": len(content),
                        "maxFileBytes": 1048576,
                        "maxResponseBytes": 1507328,
                    },
                ),
            )
            return
        if name == "read_binary_range":
            target = safe_path(str(arguments.get("path", "")))
            offset = arguments.get("offset_bytes")
            length = arguments.get("length_bytes")
            if (
                target is None
                or target.is_symlink()
                or not target.is_file()
                or isinstance(offset, bool)
                or not isinstance(offset, int)
                or offset < 0
                or offset > 67108864
                or isinstance(length, bool)
                or not isinstance(length, int)
                or length < 1
                or length > 262144
            ):
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Binary range invalid."),
                )
                return
            try:
                descriptor = os.open(
                    target,
                    os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                )
                with os.fdopen(descriptor, "rb") as stream:
                    metadata = os.fstat(stream.fileno())
                    if not stat.S_ISREG(metadata.st_mode):
                        raise OSError("binary range target is not regular")
                    if metadata.st_size > 67108864:
                        self.send_json(
                            413,
                            rpc_error(
                                identifier,
                                -32001,
                                "Payload too large",
                                "Binary range file too large.",
                            ),
                        )
                        return
                    if offset > metadata.st_size:
                        self.send_json(
                            400,
                            rpc_error(
                                identifier,
                                -32602,
                                "Invalid params",
                                "Binary range invalid.",
                            ),
                        )
                        return
                    stream.seek(offset)
                    content = stream.read(length)
                    post_metadata = os.fstat(stream.fileno())
                    if post_metadata.st_size != metadata.st_size:
                        self.send_json(
                            409,
                            rpc_error(
                                identifier,
                                -32004,
                                "Resource changed",
                                "Binary range file changed.",
                            ),
                        )
                        return
            except OSError:
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Binary range invalid."),
                )
                return
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "encoding": "base64",
                        "data": base64.b64encode(content).decode("ascii"),
                        "offsetBytes": offset,
                        "sizeBytes": len(content),
                        "fileSizeBytes": metadata.st_size,
                        "eof": offset + len(content) >= metadata.st_size,
                        "maxReadBytes": 262144,
                        "maxFileBytes": 67108864,
                        "maxResponseBytes": 393216,
                    },
                ),
            )
            return
        if name == "read_text_range":
            target = safe_path(str(arguments.get("path", "")))
            offset = arguments.get("offset_bytes")
            maximum = arguments.get("max_bytes")
            if (
                target is None
                or target.is_symlink()
                or not target.is_file()
                or isinstance(offset, bool)
                or not isinstance(offset, int)
                or offset < 0
                or offset > 67108864
                or isinstance(maximum, bool)
                or not isinstance(maximum, int)
                or maximum < 4
                or maximum > 262144
            ):
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Text range invalid."),
                )
                return
            try:
                descriptor = os.open(
                    target,
                    os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                )
                with os.fdopen(descriptor, "rb") as stream:
                    metadata = os.fstat(stream.fileno())
                    if not stat.S_ISREG(metadata.st_mode):
                        raise OSError("text range target is not regular")
                    if metadata.st_size > 67108864:
                        self.send_json(
                            413,
                            rpc_error(
                                identifier,
                                -32001,
                                "Payload too large",
                                "Text range file too large.",
                            ),
                        )
                        return
                    if offset > metadata.st_size:
                        self.send_json(
                            400,
                            rpc_error(
                                identifier,
                                -32602,
                                "Invalid params",
                                "Text range invalid.",
                            ),
                        )
                        return
                    stream.seek(offset)
                    content_bytes = stream.read(maximum)
                    post_metadata = os.fstat(stream.fileno())
                    if post_metadata.st_size != metadata.st_size:
                        self.send_json(
                            409,
                            rpc_error(
                                identifier,
                                -32004,
                                "Resource changed",
                                "Text range file changed.",
                            ),
                        )
                        return
            except OSError:
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Text range invalid."),
                )
                return
            if content_bytes and content_bytes[0] & 0xC0 == 0x80:
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Text range invalid."),
                )
                return
            physical_end = offset + len(content_bytes)
            try:
                content = content_bytes.decode("utf-8")
            except UnicodeDecodeError as error:
                if error.reason == "unexpected end of data" and physical_end < metadata.st_size:
                    content_bytes = content_bytes[: error.start]
                    content = content_bytes.decode("utf-8")
                else:
                    self.send_json(
                        400,
                        rpc_error(
                            identifier,
                            -32602,
                            "Invalid params",
                            "Text range encoding invalid.",
                        ),
                    )
                    return
            next_offset = offset + len(content_bytes)
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "content": content,
                        "offsetBytes": offset,
                        "nextOffsetBytes": next_offset,
                        "sizeBytes": len(content_bytes),
                        "fileSizeBytes": metadata.st_size,
                        "eof": next_offset >= metadata.st_size,
                        "maxReadBytes": 262144,
                        "maxFileBytes": 67108864,
                        "maxResponseBytes": 1703936,
                    },
                ),
            )
            return
        if name == "read_file":
            target = safe_path(str(arguments.get("path", "")))
            if target is None or not target.is_file():
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Safe-root rejection."),
                )
                return
            if target.stat().st_size >= 200_000:
                self.send_json(
                    413,
                    rpc_error(
                        identifier,
                        -32001,
                        "Payload too large",
                        "File content exceeds the staged read_file response byte limit.",
                    ),
                )
                return
            content = target.read_text()
            self.send_json(
                200,
                result(
                    identifier,
                    {"path": str(target), "content": content, "size": len(content.encode())},
                ),
            )
            return
        if name == "path_metadata":
            target = safe_path(str(arguments.get("path", "")))
            if target is None or (not target.is_file() and not target.is_dir()):
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Safe-root rejection."),
                )
                return
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "path": str(target),
                        "kind": "directory" if target.is_dir() else "regular_file",
                        "sizeBytes": None if target.is_dir() else target.stat().st_size,
                        "modified": "2026-01-01T00:00:00+00:00",
                        "maxResponseBytes": 16384,
                    },
                ),
            )
            return
        if name == "search_text":
            target = safe_path(str(arguments.get("path", "")))
            query = arguments.get("query")
            if target is None or not target.is_dir() or not isinstance(query, str) or not query:
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Search arguments invalid."),
                )
                return
            matches = []
            files_scanned = 0
            bytes_scanned = 0
            entries_examined = 0
            skipped_oversized_files = 0
            skipped_invalid_utf8_files = 0
            skipped_unsafe_entries = 0
            skipped_unreadable_entries = 0
            truncated = False
            match_limit_reached = False
            for child in sorted(target.iterdir(), key=lambda item: str(item)):
                if is_write_quarantine_component(child.name):
                    continue
                entries_examined += 1
                if child.is_symlink():
                    skipped_unsafe_entries += 1
                    continue
                if not child.is_file():
                    continue
                try:
                    size = child.stat().st_size
                except OSError:
                    skipped_unreadable_entries += 1
                    continue
                if size > 1048576 or size > 8388608 - bytes_scanned:
                    skipped_oversized_files += 1
                    truncated = True
                    continue
                try:
                    raw_content = child.read_bytes()
                except OSError:
                    skipped_unreadable_entries += 1
                    continue
                if len(raw_content) > 1048576 or len(raw_content) > 8388608 - bytes_scanned:
                    skipped_oversized_files += 1
                    truncated = True
                    continue
                files_scanned += 1
                bytes_scanned += len(raw_content)
                try:
                    content = raw_content.decode("utf-8")
                except UnicodeDecodeError:
                    skipped_invalid_utf8_files += 1
                    continue
                for line_number, line in enumerate(content.split("\n"), start=1):
                    start = 0
                    while True:
                        column = line.find(query, start)
                        if column < 0:
                            break
                        if len(matches) >= 256:
                            truncated = True
                            match_limit_reached = True
                            break
                        matches.append(
                            {
                                "path": str(child),
                                "lineNumber": line_number,
                                "columnByte": len(line[:column].encode()) + 1,
                            }
                        )
                        start = column + len(query)
                    if match_limit_reached:
                        break
                if match_limit_reached:
                    break
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "path": str(target),
                        "matches": matches,
                        "truncated": truncated,
                        "entriesExamined": entries_examined,
                        "filesScanned": files_scanned,
                        "bytesScanned": bytes_scanned,
                        "skippedOversizedFiles": skipped_oversized_files,
                        "skippedInvalidUtf8Files": skipped_invalid_utf8_files,
                        "skippedUnsafeEntries": skipped_unsafe_entries,
                        "skippedUnreadableEntries": skipped_unreadable_entries,
                        "queryBytes": len(query.encode()),
                        "maxDepth": int(arguments.get("max_depth", 5)),
                        "maxEntries": 8192,
                        "maxFiles": 4096,
                        "maxFileBytes": 1048576,
                        "maxTotalBytes": 8388608,
                        "maxMatches": 256,
                        "maxResponseBytes": 262144,
                    },
                ),
            )
            return
        if name == "write_file":
            raw_path = arguments.get("path")
            content = arguments.get("content")
            dry_run = arguments.get("dry_run", True)
            if (
                not isinstance(raw_path, str)
                or not isinstance(content, str)
                or not isinstance(dry_run, bool)
            ):
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Tool arguments are invalid."),
                )
                return
            content_bytes = content.encode("utf-8")
            if not dry_run and not WRITE_FILE_CAPABILITY_ENABLED:
                self.send_json(
                    403,
                    capability_error(identifier, "write_file_mutation_disabled"),
                )
                return
            if not dry_run and grant is None:
                self.send_json(403, capability_error(identifier, "capability_grant_missing"))
                return

            response_preview = result(
                identifier,
                {
                    "dryRun": dry_run,
                    "sizeBytes": len(content_bytes),
                    "disposition": "replace",
                    "mode": "0600",
                    "maxFileBytes": MAX_WRITE_FILE_BYTES,
                    "maxResponseBytes": MAX_WRITE_FILE_RESPONSE_BYTES,
                    "recoveryArtifactRetained": False,
                },
            )
            if len(payload_bytes(response_preview)) > MAX_WRITE_FILE_RESPONSE_BYTES:
                error = rpc_error(
                    identifier,
                    -32001,
                    "Payload too large",
                    "File write response exceeds the staged response byte limit.",
                )
                if len(payload_bytes(error)) > MAX_WRITE_FILE_RESPONSE_BYTES:
                    error = rpc_error(
                        None,
                        -32001,
                        "Payload too large",
                        "File write response exceeds the staged response byte limit.",
                    )
                self.send_json(413, error)
                return

            target = safe_path(raw_path)
            if (
                target is None
                or target == SAFE_ROOT
                or not target.parent.is_dir()
                or target.is_symlink()
                or (target.exists() and not target.is_file())
            ):
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "File write invalid."),
                )
                return
            if len(content_bytes) > MAX_WRITE_FILE_BYTES:
                self.send_json(
                    413,
                    rpc_error(
                        identifier,
                        -32001,
                        "Payload too large",
                        "File content exceeds the staged write_file byte limit.",
                    ),
                )
                return
            disposition = "replace" if target.exists() else "create"
            if not dry_run:
                denial = consume_fixture_grant(
                    grant,
                    "write_file",
                    SESSION_ID,
                    target,
                    content_bytes,
                    disposition,
                )
                if denial is not None:
                    self.send_json(403, capability_error(identifier, denial))
                    return
                recovery_artifact_retained = publish_fixture_write(
                    target, content_bytes, disposition
                )
            else:
                recovery_artifact_retained = False
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "dryRun": dry_run,
                        "sizeBytes": len(content_bytes),
                        "disposition": disposition,
                        "mode": "0600",
                        "maxFileBytes": MAX_WRITE_FILE_BYTES,
                        "maxResponseBytes": MAX_WRITE_FILE_RESPONSE_BYTES,
                        "recoveryArtifactRetained": recovery_artifact_retained,
                    },
                ),
            )
            return

        self.send_json(
            501,
            rpc_error(identifier, -32601, "Method not found", "Tool unavailable."),
        )


if POSTURE == "issue-create":
    print(issue_fixture_grant("create_directory"))
    raise SystemExit(0)

if POSTURE == "issue-write":
    print(issue_fixture_grant("write_file"))
    raise SystemExit(0)

if POSTURE not in {"default", "mcp", "volume-control"}:
    raise SystemExit(2)

ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()

#!/usr/bin/env python3
"""Deterministic HTTP fixture for termux_release_validate.sh shell tests."""

from __future__ import annotations

import base64
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
VOLUME_CONTROL_COMPILED = POSTURE == "volume-control"
PORT = int(runtime_value("MCP__SERVER__PORT", "0") or "0")
TOKEN = runtime_value("MCP__AUTH__STATIC_TOKEN")
SAFE_ROOT_VALUE = runtime_value("MCP__FILE__SAFE_ROOTS")
if TOKEN is None or SAFE_ROOT_VALUE is None:
    raise SystemExit(2)
SAFE_ROOT = pathlib.Path(SAFE_ROOT_VALUE).resolve()
MAX_BODY = int(runtime_value("MCP__TRANSPORT__MAX_BODY_BYTES", "1024") or "1024")
SSE_ENABLED = runtime_value("MCP__TRANSPORT__SSE_ENABLED", "false") == "true"
SESSION_ID = "fixture-session-00000000"
CAPABILITY_ENABLED = (
    runtime_value("MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED", "false") == "true"
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
    "search_text",
    "write_file",
]


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


def safe_path(raw: str) -> pathlib.Path | None:
    try:
        candidate = pathlib.Path(raw)
        if candidate.is_symlink():
            return None
        resolved_parent = candidate.parent.resolve(strict=True)
        resolved = resolved_parent / candidate.name
        if os.path.commonpath((str(SAFE_ROOT), str(resolved))) != str(SAFE_ROOT):
            return None
        return resolved
    except (OSError, ValueError):
        return None


def grant_binding(session_id: str, target: pathlib.Path) -> bytes:
    relative = target.relative_to(SAFE_ROOT)
    root_stat = SAFE_ROOT.stat()
    key = bytes.fromhex(CAPABILITY_KEY_HEX)
    principal = hmac.new(
        key,
        b"termux-mcp:static-principal:v1\0" + TOKEN.encode(),
        hashlib.sha256,
    ).digest()
    digest = hashlib.sha256()
    digest.update(b"termux-mcp-release-fixture:create-directory:v1\0")
    for value in (principal, session_id.encode()):
        digest.update(struct.pack(">I", len(value)))
        digest.update(value)
    digest.update(struct.pack(">QQ", root_stat.st_dev, root_stat.st_ino))
    for component in relative.parts:
        encoded = os.fsencode(component)
        digest.update(struct.pack(">I", len(encoded)))
        digest.update(encoded)
    digest.update(b"filesystem.create-directory\0mutating")
    return digest.digest()


def issue_fixture_grant() -> str:
    if not CAPABILITY_ENABLED:
        raise SystemExit(2)
    try:
        key = bytes.fromhex(CAPABILITY_KEY_HEX)
    except ValueError as error:
        raise SystemExit(2) from error
    if len(key) != 32 or not CAPABILITY_KEY_ID:
        raise SystemExit(2)
    session_id = os.environ.get("MCP__CAPABILITY__SESSION_ID", "")
    target = safe_path(os.environ.get("MCP__CAPABILITY__CREATE_DIRECTORY_TARGET", ""))
    if not session_id or target is None or target.exists() or not target.parent.is_dir():
        raise SystemExit(2)
    issued = int(time.time())
    payload = (
        os.urandom(16)
        + grant_binding(session_id, target)
        + struct.pack(">QQ", issued, issued + 60)
        + bytes(66)
    )
    if len(payload) != 130:
        raise SystemExit(2)
    signed = f"v1.{CAPABILITY_KEY_ID}.{payload.hex()}"
    signature = hmac.new(key, signed.encode(), hashlib.sha256).hexdigest()
    return f"{signed}.{signature}"


def consume_fixture_grant(
    raw: str | None, session_id: str, target: pathlib.Path
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
    if len(payload) != 130 or len(signature) != 32 or len(key) != 32:
        return "capability_grant_malformed"
    signed = f"{version}.{key_id}.{payload_hex}".encode()
    expected = hmac.new(key, signed, hashlib.sha256).digest()
    if not hmac.compare_digest(signature, expected):
        return "capability_grant_signature_invalid"
    grant_id = payload[:16]
    binding = payload[16:48]
    issued, expires = struct.unpack(">QQ", payload[48:64])
    current = int(time.time())
    if binding != grant_binding(session_id, target):
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
            not value.isascii() or not value or len(value) > 384
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
                    tools.append(
                        {
                            "name": name,
                            "description": (
                                "Fixture create_directory requires MCP-Capability-Grant "
                                "for explicit mutation."
                            ),
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": {"type": "string"},
                                    "dry_run": {"type": "boolean"},
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
        if grant is not None and name != "create_directory":
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
                        "fileHashing": True,
                        "fileHashAlgorithm": "sha256",
                        "fileHashMaxBytes": 16777216,
                        "createDirectoryMutationEnabled": CAPABILITY_ENABLED,
                        "createDirectoryGrantRequired": CAPABILITY_ENABLED,
                        "createDirectoryGrantHeader": "mcp-capability-grant",
                        "createDirectoryGrantTtlSeconds": 60,
                        "createDirectoryMutationMode": (
                            "dry_run_or_request_scoped_single_use_grant"
                            if CAPABILITY_ENABLED
                            else "dry_run_only_mutation_disabled"
                        ),
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
                if not CAPABILITY_ENABLED:
                    self.send_json(
                        403,
                        capability_error(identifier, "create_directory_mutation_disabled"),
                    )
                    return
                denial = consume_fixture_grant(grant, SESSION_ID, target)
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
            target = safe_path(str(arguments.get("path", "")))
            if target is None:
                self.send_json(
                    400,
                    rpc_error(identifier, -32602, "Invalid params", "Safe-root rejection."),
                )
                return
            content = str(arguments.get("content", ""))
            dry_run = arguments.get("dry_run", True)
            if not dry_run:
                target.write_text(content)
                target.chmod(0o600)
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "dryRun": dry_run,
                        "bytes": len(content.encode()),
                        "message": "fixture-write",
                    },
                ),
            )
            return

        self.send_json(
            501,
            rpc_error(identifier, -32601, "Method not found", "Tool unavailable."),
        )


if POSTURE == "issue":
    print(issue_fixture_grant())
    raise SystemExit(0)

if POSTURE not in {"default", "mcp", "volume-control"}:
    raise SystemExit(2)

ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()

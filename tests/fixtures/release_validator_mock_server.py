#!/usr/bin/env python3
"""Deterministic HTTP fixture for termux_release_validate.sh shell tests."""

from __future__ import annotations

import json
import os
import pathlib
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any


POSTURE = sys.argv[1]
VERSION = sys.argv[2]
PORT = int(os.environ["MCP__SERVER__PORT"])
TOKEN = os.environ["MCP__AUTH__STATIC_TOKEN"]
SAFE_ROOT = pathlib.Path(os.environ["MCP__FILE__SAFE_ROOTS"]).resolve()
MAX_BODY = int(os.environ["MCP__TRANSPORT__MAX_BODY_BYTES"])
SESSION_ID = "fixture-session-00000000"
TOOLS = [
    "runtime_status",
    "platform_info",
    "android_status",
    "project_service_status",
    "create_directory",
    "list_directory",
    "path_metadata",
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
                "mcp_runtime_enabled": POSTURE == "mcp",
                "safe_root_count": 1,
                "auth_posture": "static_token",
            }
            if POSTURE == "mcp":
                ready["mcp_request_limits"] = {
                    "max_concurrent_requests": 4,
                    "request_timeout_seconds": 30,
                    "max_body_bytes": MAX_BODY,
                }
            self.send_json(200, ready)
            return
        if self.path != "/mcp" or POSTURE != "mcp":
            self.send_json(404, {"error": "not_found"})
            return
        if not self.authenticated() or not self.transport_allowed() or not self.active_session():
            return
        self.send_bytes(405)

    def do_DELETE(self) -> None:
        if self.path != "/mcp" or POSTURE != "mcp":
            self.send_json(404, {"error": "not_found"})
            return
        if not self.authenticated() or not self.transport_allowed() or not self.active_session():
            return
        self.send_bytes(204)

    def do_POST(self) -> None:
        if self.path != "/mcp" or POSTURE != "mcp":
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
        if method == "initialize":
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
            self.send_bytes(202)
            return
        if method == "tools/list":
            self.send_json(
                200,
                {
                    "jsonrpc": "2.0",
                    "id": identifier,
                    "result": {
                        "tools": [
                            {
                                "name": name,
                                "description": "fixture",
                                "inputSchema": {"type": "object"},
                            }
                            for name in TOOLS
                        ]
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
        if name == "runtime_status":
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "commandExecution": False,
                        "androidPlatformTools": False,
                        "highImpactTools": False,
                    },
                ),
            )
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
            for child in sorted(target.iterdir(), key=lambda item: str(item)):
                entries_examined += 1
                if not child.is_file() or child.is_symlink():
                    continue
                content = child.read_text()
                files_scanned += 1
                bytes_scanned += len(content.encode())
                for line_number, line in enumerate(content.split("\n"), start=1):
                    start = 0
                    while True:
                        column = line.find(query, start)
                        if column < 0:
                            break
                        matches.append(
                            {
                                "path": str(child),
                                "lineNumber": line_number,
                                "columnByte": len(line[:column].encode()) + 1,
                            }
                        )
                        start = column + len(query)
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "path": str(target),
                        "matches": matches,
                        "truncated": False,
                        "entriesExamined": entries_examined,
                        "filesScanned": files_scanned,
                        "bytesScanned": bytes_scanned,
                        "skippedOversizedFiles": 0,
                        "skippedInvalidUtf8Files": 0,
                        "skippedUnsafeEntries": 0,
                        "skippedUnreadableEntries": 0,
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


if POSTURE not in {"default", "mcp"}:
    raise SystemExit(2)

ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()

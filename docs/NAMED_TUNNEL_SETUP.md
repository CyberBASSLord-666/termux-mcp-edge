# Named Cloudflare Tunnel Setup

`scripts/setup_named_tunnel.sh` is the project-scoped helper for a locally managed Cloudflare Tunnel. It never selects a tunnel name or hostname by default, never overwrites DNS, and never changes the MCP listener, authentication posture, or runit service.

## Supported command contract

The helper requires a `cloudflared` release that supports:

```text
cloudflared tunnel list --output json
cloudflared tunnel create NAME
cloudflared tunnel route dns TUNNEL HOSTNAME
cloudflared tunnel run TUNNEL
```

It also requires `jq` for strict JSON validation. Tunnel discovery uses the structured JSON list and an exact `.name` comparison. A list/authentication/network failure is never interpreted as an absent tunnel.

DNS provisioning deliberately omits `--overwrite-dns`. A route command failure—including a hostname already owned by another tunnel or record type—stops with a non-sensitive error and requires operator review in Cloudflare DNS. The helper does not parse human-readable account output or guess ownership from substring matches.

## Preflight

Configure and test static bearer authentication, exact Host/Origin allowlists for the public hostname, request limits, and the supervised local service before creating any tunnel or DNS record. Do not use localhost-only unauthenticated mode through a tunnel.

Validate the plan without calling `cloudflared`:

```bash
bash scripts/setup_named_tunnel.sh --dry-run termux-mcp mcp.example.com
```

The command validates the exact tunnel name and DNS hostname, reports that creation is not authorized, and makes no external calls.

## Existing tunnel

Reuse an exact existing tunnel and create its non-overwriting DNS route:

```bash
bash scripts/setup_named_tunnel.sh termux-mcp mcp.example.com
```

If the tunnel is absent, the helper fails without login or creation.

## Explicit creation

Authenticate separately with Cloudflare's documented `cloudflared tunnel login` flow, then verify `cloudflared tunnel list --output json` succeeds. This keeps any interactive authentication URL or account output outside the helper and its logs.

After reviewing the external account mutation, explicitly authorize creation:

```bash
bash scripts/setup_named_tunnel.sh --create termux-mcp mcp.example.com
```

The helper never starts authentication. If inventory cannot be read, it fails without mutation even when `--create` is supplied. After creation it re-lists and requires exactly one exact-name match before attempting DNS.

The final printed `cloudflared tunnel run` command is informational. Integrating the tunnel into runit remains a separate reviewed service change.

## Validation and failure behavior

Run the hermetic suite:

```bash
bash -n scripts/setup_named_tunnel.sh tests/setup_named_tunnel_test.sh
bash tests/setup_named_tunnel_test.sh
```

Coverage includes missing arguments, malformed hostnames, no-call dry-run, inventory/auth failure, absent tunnels without creation permission, unsupported JSON, exact existing-tunnel reuse, DNS conflict without overwrite, create failure, post-create confirmation, and private temporary-file cleanup.

Temporary command output is stored under an owner-only directory and removed on success, failure, or interruption. Raw Cloudflare output is not copied to standard output or error, reducing accidental disclosure of account or credential details.

# Secure Rust Embedding

The `mcp-runtime` feature exposes one public MCP construction path:
`mcp_transport::McpRouterBuilder`. It returns a complete protected router, not a
raw `/mcp` route. Legacy router constructors, raw transport state, transport
options, and capability-authority bundles are not public API.

## Minimal static-token embedding

Bind the listener first and give that exact listener to the builder. Serve the
result with `ConnectInfo<McpConnectionInfo>` derived by Axum from each accepted
TCP stream; this is mandatory even when static-token authentication is used
because it keeps the same router valid under the strict localhost-development
posture.

```rust,no_run
use std::path::PathBuf;

use termux_mcp_server::{
    auth::{McpAuthPolicy, McpConnectionInfo},
    mcp_transport::McpRouterBuilder,
    request_limits::McpRequestLimits,
    transport_security::TransportSecurityPolicy,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8000").await?;
    let auth = McpAuthPolicy::static_bearer("replace-with-a-strong-token")?;
    let limits = McpRequestLimits::from_seconds(4, 30, 2 * 1024 * 1024)?;
    let transport = TransportSecurityPolicy::localhost(8000, false)?;

    let app = McpRouterBuilder::try_new(
        &listener,
        auth,
        limits,
        transport,
        vec![PathBuf::from("/absolute/private/safe-root")],
    )?
    .with_sse_enabled(false)
    .build()?;

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<McpConnectionInfo>(),
    )
    .await?;
    Ok(())
}
```

`try_new` reads the address from the already-bound socket and validates and
lifetime-pins every safe root. It rejects an empty set, relative paths,
filesystem root, missing objects, non-directories, and any symlink in a root or
its ancestors. Errors use `McpRouterBuildError` and do not include bearer
tokens, configured paths, descriptor numbers, or filesystem identities.

`McpAuthPolicy` is opaque: downstream code can construct a validated static or
localhost policy, clone it, and use its redacted `Debug` implementation, but it
cannot destructure the policy to recover the bearer principal.
`McpConnectionInfo` is also opaque: Axum constructs it from the accepted TCP
stream, while downstream code cannot synthesize or inspect trusted peer/local
fields. This keeps credentials and socket identity inside the authentication
and startup authority-matching boundary.

## Mandatory order

The builder installs one non-configurable outer boundary in this order:

1. static bearer authentication, or actual-peer loopback authentication in the
   explicit development posture;
2. authenticated `Content-Length` rejection, fail-fast concurrency admission,
   and the total request timeout;
3. streaming request-body enforcement and body extraction;
4. exact `Host` and browser `Origin` validation;
5. HTTP method and media negotiation, JSON-RPC classification, protocol and
   session lifecycle, discovery, grant-context handling, tool dispatch, and
   any mutation work.

An unauthenticated request therefore cannot allocate a session, consume an MCP
concurrency permit, obtain body-limit details, parse a grant, discover a tool,
read a safe root, or enter mutation preparation. Embeddings cannot reorder or
omit these layers because no raw public router constructor exists.

## Localhost-only development

`McpAuthPolicy::unauthenticated_localhost_only()` is accepted only when the
listener passed to `try_new` is actually bound to an IPv4 or IPv6 loopback
address. Each request must independently contain Axum
`ConnectInfo<McpConnectionInfo>` derived from its accepted TCP stream. The
actual peer must be loopback and the stream's local address must exactly match
the listener validated by the builder. Missing metadata, a wildcard/non-loopback
listener, a non-loopback peer, or listener substitution fails closed. `Host`,
`Origin`, and forwarded headers are not socket evidence.

Do not use this posture behind a proxy, tunnel, LAN listener, or adapter that
does not preserve socket peer metadata. Prefer a static bearer token for every
shared or remotely reachable deployment.

## Optional capabilities

Read-only optional capabilities are selected before `build`:

```rust,ignore
let builder = builder
    .with_android_battery_status_enabled(true)
    .with_android_volume_status_enabled(true);
```

Requesting a client that was not compiled into the selected feature posture
returns `McpRouterBuildError::CapabilityUnavailable`; it is never silently
disabled. SSE is likewise explicit and defaults off.

The `full-suite` feature only makes every supported optional implementation
available to the package build. It does not call any builder opt-in, enable a
runtime flag, create a mutation authority, or satisfy a request grant. A
full-suite process therefore has the same 17-tool discovery baseline until its
four optional runtime gates are enabled independently; with all four enabled it
has exactly 21 tools. Raw Cargo `--all-features` remains a development
compatibility posture, not an embedding or public artifact contract.

Mutation authorities are added with the fallible
`try_with_create_directory_authority`, `try_with_copy_file_authority`,
`try_with_trash_file_authority`, `try_with_write_file_authority`, and—when compiled—
`try_with_android_volume_control_authority` methods. Each authority must be
cryptographically bound to the exact static-bearer principal used by the
builder. A mismatched principal, or any mutation authority in unauthenticated
development mode, is rejected before a router exists. These checks complement;
they do not replace the runtime gates, active-session binding, exact target
binding, single-use grant consumption, and mutation-specific verification.

Command execution is intentionally not enableable through the public builder.
Only the package binary can select its crate-private fixed-profile command
posture. A dependency consumer cannot obtain raw command clients, profiles,
transport state, legacy router constructors, or an authority that turns on the
command lane.

`filesystem_tools()` returns a clone of the exact pinned filesystem authority
owned by the future router. Use it only where an embedding must share that
identity with other project code or offline grant issuance. Public mutation
methods, including `trash_file`, remain preview-only; live preparation and
execution are crate-private to the grant-aware transport. Do not independently
reconstruct safe-root tools and assume their authority identity is
interchangeable.

## Composition rules

- Serve the same listener supplied to `try_new` and always use
  `into_make_service_with_connect_info::<McpConnectionInfo>()`. A different
  served listener fails authentication even when it is also loopback-bound.
- Merge unrelated health/readiness routes only outside `/mcp`; never overlay or
  replace `/mcp` after the builder returns.
- Construct `McpAuthPolicy`, `McpRequestLimits`, and
  `TransportSecurityPolicy` with their fallible validated constructors.
- Treat `build` failure as a startup failure. Do not retry by dropping a
  requested capability or weakening a policy.
- Keep safe roots under exclusive service ownership whenever live mutation
  gates are enabled.
- Never log `Debug` output from surrounding application state if that state may
  independently contain credentials. The project builder itself redacts its
  authentication material and filesystem authority.

The shipped binary follows this same builder path. Its only additional access
is the crate-private command-profile switch, which downstream crates cannot
name or call.

The checked-in [`secure_embedding` example](../examples/secure_embedding.rs)
is compiled by the repository's default, minimal-`mcp-runtime`, named
`full-suite`, and raw all-feature gates. To run it deliberately, provide a private token and an existing absolute
safe root through `MCP_EXAMPLE_STATIC_TOKEN` and `MCP_EXAMPLE_SAFE_ROOT`, then
use `cargo run --locked --example secure_embedding --features mcp-runtime`.

# Secure MCP embedding

Downstream applications have one supported MCP router construction path: `McpRouterBuilder`. The builder requires every policy that protects `/mcp`, validates and lifetime-pins the filesystem boundary before it can return, and produces the same protected router used by the package binary. Raw transport state and legacy router constructors are not public API.

Enable the `mcp-runtime` feature in the embedding crate:

```toml
termux-mcp-server = { version = "0.6.0", features = ["mcp-runtime"] }
```

## Minimal static-token server

Build the complete router before opening the listener. Always serve it with Axum `ConnectInfo<SocketAddr>`; unauthenticated localhost policy uses that request-time peer metadata and fails closed when it is absent.

```rust,no_run
use std::{net::SocketAddr, path::PathBuf};

use termux_mcp_server::{
    auth::McpAuthPolicy,
    mcp_transport::McpRouterBuilder,
    request_limits::McpRequestLimits,
    transport_security::TransportSecurityPolicy,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    const PORT: u16 = 8_000;

    let auth = McpAuthPolicy::static_bearer(std::env::var("MCP_TOKEN")?)?;
    let limits = McpRequestLimits::from_seconds(
        4,                 // fail-fast concurrent-request ceiling
        30,                // total request timeout
        2 * 1024 * 1024,   // streaming request-body ceiling
    )?;
    let transport = TransportSecurityPolicy::localhost(PORT, true)?;

    let router = McpRouterBuilder::new(
        "127.0.0.1",
        auth,
        limits,
        transport,
        vec![PathBuf::from(
            "/data/data/com.termux/files/home/mcp-files",
        )],
    )?
    .build()?;

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await?;
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
```

The listener host passed to `new` is a declaration the builder validates; it is not a substitute for binding the intended address. Keep the declaration and `TcpListener` address identical.

## Exact request order

The returned router fixes the following outer-to-inner execution order:

1. Bearer authentication, or request-time loopback-peer proof for explicit unauthenticated localhost mode.
2. Early `Content-Length` rejection, fail-fast concurrency acquisition, and the total request timeout.
3. Streaming body-size enforcement and body extraction.
4. Exact `Host` and browser `Origin` policy.
5. HTTP method, media-type, protocol-version, session, and capability-grant validation.
6. JSON-RPC parsing, lifecycle and discovery handling, tool dispatch, and any authorized mutation.

This order is part of the public security contract. An unauthenticated request cannot consume a request permit, start the timeout-protected inner future, read its body, reveal body-limit details, create or inspect a session, discover tools, process grants, read a safe root, or reach a mutation.

## Authentication postures

`McpAuthPolicy` has private representation and can be created only through validated constructors:

- `McpAuthPolicy::static_bearer(...)` validates a bounded non-empty ASCII-graphic token. Its `Debug` output redacts the token.
- `McpAuthPolicy::from_config(...)` derives the same sealed policy from validated server configuration.
- `McpAuthPolicy::unauthenticated_localhost_only()` is an explicit development posture. `McpRouterBuilder::new` rejects a non-loopback declared listener, and middleware independently requires an actual IPv4 or IPv6 loopback peer from `ConnectInfo<SocketAddr>`. Missing metadata and non-loopback peers fail closed before limits or body handling.

Do not remove or replace the builder's route layers. If the router is nested into a larger application, preserve `ConnectInfo<SocketAddr>` at the serving boundary and do not expose a second unprotected `/mcp` route.

## Construction failures

`McpRouterBuilder::new` and `build` return `McpRouterBuildError`; neither uses embedding configuration as a panic path.

- `InvalidListenerHost` rejects an empty, padded, control-bearing, or whitespace-bearing listener declaration.
- `UnauthenticatedListenerRequiresLoopback` rejects development auth with a non-loopback declaration.
- `SafeRoots(...)` reports bounded safe-root configuration failures without exposing path, descriptor, or inode details. An empty set, relative path, filesystem root, missing path, non-directory, symlinked component, or more than 64 input entries is rejected before a router or listener exists.
- `CapabilityNotCompiled { capability }` rejects a requested optional runtime gate when its Cargo feature is absent.
- `CapabilityRequiresStaticAuthentication { capability }` rejects every create/copy/write or Android-volume mutation authority paired with unauthenticated localhost policy.
- `OptionalClientUnavailable { client }` rejects a requested provider or command client that cannot be initialized safely.

Safe roots are normalized, opened component-by-component without following links, and retained as pinned directory descriptors for the router lifetime. Renaming or replacing a configured pathname cannot redirect a running embedding. Builder clones do not reopen roots because the builder is consumed by `build`.

## Optional capabilities

All optional runtime gates default to disabled. Enable only a capability compiled into the embedding:

```rust,ignore
let builder = McpRouterBuilder::new(listener_host, auth, limits, transport, roots)?
    .with_transport_options(options)
    .with_android_battery_status_enabled(enable_battery)
    .with_android_volume_status_enabled(enable_volume)
    .with_create_directory_authority(create_authority)
    .with_copy_file_authority(copy_authority)
    .with_write_file_authority(write_authority);

// Available only with the android-volume-control feature:
let builder = builder.with_android_volume_control_authority(volume_authority);

let router = builder.build()?;
```

Transport options include the independently default-disabled bounded SSE response posture. Mutation-authority setters accept already validated authorities. `build` rejects every mutation authority unless the sealed policy is static bearer authentication; unauthenticated localhost mode can never activate create, copy, write, or Android-volume mutation. An authority also does not bypass each capability's runtime gate, request-bound grant, dry-run, replay, identity, and safe-root checks.

The fixed-profile command diagnostic lane is intentionally unavailable to downstream crates. The package binary uses this same builder and a crate-private setter, so public embeddings cannot expand their authority into command execution even when the dependency is compiled with `command-execution`.

## Embedding validation checklist

Before deployment:

1. Construct policies and the router before binding the listener.
2. Bind exactly the host and port represented by the listener declaration and transport policy.
3. Serve with `into_make_service_with_connect_info::<SocketAddr>()`.
4. Verify missing and incorrect bearer tokens return HTTP 401 for POST, GET, and DELETE, including oversized bodies.
5. If localhost development mode is required, verify missing `ConnectInfo` and a non-loopback peer fail closed.
6. Verify invalid safe roots and uncompiled optional gates return typed construction errors.
7. Run default-feature and all-feature format, Clippy, and test gates, then the Android and native Termux gates documented in [Validation](VALIDATION.md).

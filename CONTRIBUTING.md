# Contributing

This repository is intended to be operated as a security-sensitive Android edge MCP server. Treat every change as production-impacting.

## Development workflow

1. Create a feature branch from `main`.
2. Keep changes focused and reviewable.
3. Run local validation before opening a pull request:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

4. For Android release validation, run:

```bash
./scripts/cross_compile.sh
```

## Security expectations

- Do not commit secrets, tokens, tunnel credentials, certificates, private keys, or device-specific configuration.
- New tools must declare their risk profile and minimum required scope.
- Any tool that mutates local files, launches commands, interacts with Android automation, or accesses the network must be disabled by default or protected by explicit scope checks.
- Path-taking code must canonicalize or safely resolve paths and enforce configured safe roots.
- Network-taking code must reject localhost, link-local, private-address, and metadata-service targets unless explicitly and narrowly allowed.

## Documentation expectations

Every behavioral change should update at least one of:

- `README.md`
- `docs/SECURITY.md`
- `docs/OPERATIONS.md`
- `CHANGELOG.md`

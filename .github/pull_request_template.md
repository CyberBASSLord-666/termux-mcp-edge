## Summary

Describe the change and why it is needed.

## Production readiness checklist

- [ ] Exact-head CI passed.
- [ ] Exact-head Security passed.
- [ ] Dependency alerts were checked after the change.
- [ ] No vulnerable or unused dependency surface was introduced.
- [ ] Documentation matches the compiled runtime behavior.
- [ ] Runtime behavior is covered by tests or a smoke-test note.

## Transport and tool-surface checklist

Complete this section for any change that exposes MCP transport, filesystem tools, platform tools, network access, or other agent-callable actions.

- [ ] Host and Origin or equivalent anti-rebinding protections are documented and tested.
- [ ] Authentication behavior is explicit for loopback and non-loopback listeners.
- [ ] Authorization and operator-consent assumptions are documented.
- [ ] High-impact tools are feature-gated or otherwise deliberately scoped.
- [ ] At least one safe MCP tool-discovery smoke test is included.
- [ ] At least one safe MCP tool-call smoke test is included.

## Validation evidence

Paste the exact head SHA and validation result links or run IDs here before merge.

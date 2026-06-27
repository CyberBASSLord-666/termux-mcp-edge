# Security Best Practices for Termux MCP Server

This document outlines the security considerations and recommended countermeasures when operating a Model Context Protocol (MCP) server on Android using Termux.  The information is based on industry guidance and best‑practice research.

## 1. Threat Landscape

### Unauthenticated Endpoints and Remote Code Execution

Misconfigured MCP servers frequently expose endpoints that lack proper authentication.  A notable example was a critical CVE (CVE‑2025‑49596) in the Anthropic MCP Inspector; the vulnerability stemmed from **unauthenticated communication between the client and proxy**, which allowed remote code execution simply by luring a developer to a malicious website【817876964395207†L329-L347】.  The lesson is clear: **never expose unauthenticated endpoints**.

### Server‑Side Request Forgery (SSRF)

Tools that fetch user‑controlled URLs can be abused to access internal services.  Attackers may coerce an agent to fetch internal metadata endpoints, extracting sensitive cloud credentials【817876964395207†L350-L354】.  Always validate and restrict outbound network access.

## 2. Authentication and Authorization

### Use Cryptographically Verifiable Identity

An MCP server must **demand cryptographically verifiable client identity and explicit scoped authorization**.  You **cannot rely on static API keys or long‑lived session cookies**【817876964395207†L357-L361】.  The recommended pattern is to adopt modern **OAuth 2.1** with **Proof Key for Code Exchange (PKCE)**.  PKCE protects against code interception; the client generates a random secret, hashes it, and sends the hash to the authorization server.  When exchanging the authorization code for an access token, the client must present the original secret.  If an attacker intercepts the redirect, they cannot reuse the code because they do not possess the initial secret【817876964395207†L357-L374】.

Static bearer tokens are supported in this implementation for simplicity, but they should only be used in trusted, air‑gapped environments.  For enterprise deployments you should integrate the server with a dedicated authorization service (e.g. OAuth 2.1 with PKCE) and verify tokens via JSON Web Token (JWT) signatures.

### Fail Closed Without an Explicit Token

Startup requires `MCP__AUTH__STATIC_TOKEN` by default.  A missing token fails closed before the HTTP listener starts.  The only supported exception is explicit local development mode:

```bash
export MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true
export MCP__SERVER__HOST=127.0.0.1
```

This opt-in is rejected for non-loopback bind addresses.  It is unsafe for Cloudflare Tunnel, VPN, LAN, reverse-proxy, port-forwarded, shared-device, or rish-capable deployments.  Treat any unauthenticated listener as local-only and disposable.

### Reject the Token‑Passthrough Anti‑pattern

Never allow a client to perform the entire OAuth flow and simply forward an access token to the MCP server.  This **breaks the chain of trust**: the server cannot verify that the token belongs to the caller【817876964395207†L378-L392】.  The MCP server itself must manage the token exchange process and validate proofs to guarantee identity integrity.

### Fine‑Grained Scoping

Provision agents with the **minimum necessary privileges**.  The specification recommends progressive scoping and role‑based authorization.  Request only read‑only scopes on startup, and prompt the user for consent when higher privileges are required【817876964395207†L417-L429】.  Do not expose a single “god mode” token.

## 3. Filesystem and Tool Safety

### Prevent Directory Traversal

Path traversal attacks exploit untrusted input to access files outside the intended directory.  To mitigate this:

* **Validate user input**.  Ensure that paths contain only acceptable characters or match a whitelist【686811669605603†L83-L92】.
* **Canonicalize and clamp**.  After resolving symlinks, ensure the canonicalized path **starts with an expected base directory**【686811669605603†L94-L96】.  Our server implements this by canonicalizing the user‑supplied path and verifying that it begins with one of the configured safe roots.  Attempts to access `/etc`, `/data`, `/system` or any parent via `..` are denied.
* **Avoid user‑controlled filenames in low‑level APIs**.  Where possible, use indices or identifiers rather than raw file paths【686811669605603†L136-L145】.

### Symlink Containment

Symlinks can lead outside the safe root even if the user‑supplied path appears safe.  The server sanitizes every path, including those discovered during directory traversal, to prevent following symlinks outside the allowed roots.

### Restrict Network Access in Tools

When implementing tools that fetch remote URLs, maintain a strict allowlist of egress domains and reject requests to local addresses or cloud metadata endpoints【817876964395207†L490-L496】.  Harden your network layer (e.g. Cloudflare or local firewall) to block access to `169.254.169.254` and other internal IP ranges.

## 4. Environment Hardening

### Local (IDE / Developer Workstation)

Local MCP servers run directly on developer machines.  They should be isolated using containerization (Docker, gVisor, etc.) and **require un‑bypassable user consent** for any action that could access the host filesystem or network【817876964395207†L440-L462】.  A compromised agent must not be able to read SSH keys or pivot into a VPN.

### Remote (Enterprise Deployment)

For enterprise deployments, position an **API gateway** (such as Tyk) in front of the MCP server.  Enforce mutual TLS (mTLS) for service‑to‑service communication, apply strict rate limits, and inject secrets via a central vault at execution time【817876964395207†L466-L482】.  Never store long‑lived credentials in the MCP server’s environment.

## 5. Android‑Specific Reliability

Android aggressively terminates background processes to conserve resources.  To maintain a reliable Termux MCP server:

* **Disable phantom process killing**.  On Android 14 or newer, enable the toggle at **Settings → System → Developer options → Disable child process restrictions** to stop the system from terminating background processes; this may require unlocking developer options first【48348016950568†L254-L271】.  On older versions, you can use `adb` or `su` commands to set `settings_enable_monitor_phantom_procs` to `false`【48348016950568†L274-L287】.
* **Configure battery settings**.  Go to **Settings → Apps → Termux → Battery** and enable **Allow background activity**.  Then open Termux and tap **Acquire wakelock** in the notification【252442380034806†L140-L149】.
* **Use wakelocks sparingly**.  Holding a wake lock prevents the device from sleeping but dramatically affects battery life.  Only hold the wake lock when necessary and release it as soon as possible【402006191980019†L497-L512】.

## 6. Additional Hardening

* **Disable RAM Plus** on Samsung devices to prevent storage wear during heavy paging.
* **Disable Samsung Auto Blocker** to allow side‑loaded binaries and CLI networking.
* **Run under supervision**.  Use `termux-services` with runit to supervise the MCP server.  Create a service directory in `$PREFIX/var/service/` and write a `run` script that sets environment variables (host, port, auth token) and executes the server binary.  You can enable the service at boot with `sv-enable` and start it with `sv up`【725050930974417†L52-L109】.
* **Use Cloudflare Tunnels** to expose your MCP server without opening direct ports.  The provided `setup_named_tunnel.sh` script demonstrates how to create a named tunnel and route DNS accordingly.

## 7. Incident Response

Prepare an incident response plan that includes token revocation, secret rotation, and forced re‑authentication.  If you suspect compromise, invalidate all active sessions, rotate any leaked credentials, and audit logs for unauthorized access.

## Conclusion

Security is a process, not a checkbox.  The Termux MCP server ships with sensible defaults—path sanitization, safe roots, metrics, and fail-closed bearer-token posture—but secure deployments must go further.  Employ modern authentication with PKCE, harden the runtime environment, and continuously monitor and update your deployment.

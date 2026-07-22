# Working with the Canonical GitHub Repository

The existing canonical public repository is:

```text
https://github.com/CyberBASSLord-666/termux-mcp-edge
```

Do not create or import a replacement repository when the goal is to install, inspect, or contribute to this project.

## Clone the public repository

No GitHub account or authentication is required for a read-only HTTPS clone:

```bash
git clone https://github.com/CyberBASSLord-666/termux-mcp-edge.git
cd termux-mcp-edge
git remote -v
git status --short --branch
```

Verify that `origin` points to the canonical repository before using release, deployment, or validation scripts:

```bash
test "$(git remote get-url origin)" = \
  'https://github.com/CyberBASSLord-666/termux-mcp-edge.git'
git fetch --prune origin
git switch main
git pull --ff-only origin main
```

For an exact-commit workflow, obtain the full expected commit SHA from the trusted release or validation record and verify it after checkout:

```bash
EXPECTED_COMMIT='<full-40-character-commit-sha>'
git fetch --prune --tags origin
git cat-file -e "${EXPECTED_COMMIT}^{commit}"
git checkout --detach "$EXPECTED_COMMIT"
test "$(git rev-parse HEAD)" = "$EXPECTED_COMMIT"
```

Do not substitute an unverified branch name, abbreviated SHA, pull-request artifact, or unrelated fork when exact-source release evidence is required.

## Contribute through a fork

Create a fork of `CyberBASSLord-666/termux-mcp-edge` in GitHub, then clone that fork and register the canonical repository as `upstream`:

```bash
GITHUB_USER='<your-github-user>'
git clone "https://github.com/${GITHUB_USER}/termux-mcp-edge.git"
cd termux-mcp-edge
git remote add upstream https://github.com/CyberBASSLord-666/termux-mcp-edge.git
git remote -v
git fetch --prune upstream
```

Keep the fork's `main` branch synchronized without rewriting shared history:

```bash
git switch main
git merge --ff-only upstream/main
git push origin main
```

Create focused work on a topic branch and follow the repository's [contribution requirements](../CONTRIBUTING.md) before opening a pull request against the canonical `main` branch.

## Verify an existing checkout

Inspect remotes before changing them:

```bash
git remote -v
git rev-parse --show-toplevel
git status --short --branch
```

For a direct public clone, `origin` should be the canonical URL. For a contributor fork, `origin` should be the contributor's fork and `upstream` should be the canonical URL. Add or repair only the missing role:

```bash
git remote add upstream https://github.com/CyberBASSLord-666/termux-mcp-edge.git
```

If `upstream` already exists, verify its value with `git remote get-url upstream` before using `git remote set-url`. Never overwrite a contributor's `origin` merely to add the canonical read-only remote.

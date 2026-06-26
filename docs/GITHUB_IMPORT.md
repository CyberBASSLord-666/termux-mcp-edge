# GitHub Import Instructions

The preferred canonical repository is:

```text
CyberBASSLord-666/termux-mcp-edge
```

The currently available ChatGPT GitHub connector can read and update existing repositories, but it does not expose a repository-creation action. Create the repository once in GitHub, then this project can be pushed into it and used as the durable source of truth for recurring improvement passes.

## Option A — GitHub CLI

From the repository root on a machine with `gh` authenticated:

```bash
gh repo create CyberBASSLord-666/termux-mcp-edge \
  --private \
  --description "Secure Rust MCP edge server for Android Termux" \
  --source . \
  --remote origin \
  --push
```

## Option B — Manual GitHub UI

1. Go to GitHub and create a new private repository named `termux-mcp-edge` under `CyberBASSLord-666`.
2. Do not add generated sample files if you intend to push this repository directly.
3. From this project root:

```bash
git remote add origin https://github.com/CyberBASSLord-666/termux-mcp-edge.git
git branch -M main
git push -u origin main
```

## Option C — Import from bundle

If you received `termux-mcp-edge.git.bundle`:

```bash
git clone termux-mcp-edge.git.bundle termux-mcp-edge
cd termux-mcp-edge
git remote add origin https://github.com/CyberBASSLord-666/termux-mcp-edge.git
git push -u origin main
```

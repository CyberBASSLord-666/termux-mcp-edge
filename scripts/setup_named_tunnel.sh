#!/usr/bin/env bash
# Setup Named Cloudflare Tunnel for Termux MCP Server
set -euo pipefail

TUNNEL_NAME="${1:-termux-mcp}"
DOMAIN="${2:-mcp.yourdomain.com}"

echo "=== Setting up Named Cloudflare Tunnel: $TUNNEL_NAME ==="

cloudflared tunnel login
cloudflared tunnel create "$TUNNEL_NAME"
cloudflared tunnel route dns "$TUNNEL_NAME" "$DOMAIN"

echo "Tunnel created. Credentials saved."
echo "Update your runit service to use:"
echo "  cloudflared tunnel run $TUNNEL_NAME"

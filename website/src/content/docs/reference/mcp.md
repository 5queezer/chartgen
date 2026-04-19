---
title: MCP Integration
description: Connect chartgen to Claude Desktop or Claude.ai.
---

chartgen speaks the Model Context Protocol over both stdio (Claude Desktop)
and HTTP (Claude.ai). Both modes share the same tool definitions.

## Claude Desktop

Add chartgen to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "chartgen": {
      "command": "/path/to/chartgen",
      "args": ["--mcp"]
    }
  }
}
```

## Claude.ai

Run `chartgen --serve` behind a TLS-terminating reverse proxy, then add your
domain as a remote MCP connector in Claude.ai. OAuth 2.1 PKCE is handled by
chartgen — no manual token management.

## Tools

### `generate_chart`

Renders a chart and returns a base64-encoded PNG. Accepts symbol, interval,
an array of indicator specs, and image dimensions.

### `list_indicators`

Returns the full catalog of registered indicators with their accepted
parameters. Use this to discover what you can pass to `generate_chart`.

## Deployment

See the [Dockerfile](https://github.com/5queezer/chartgen/blob/master/Dockerfile)
for the 2-stage build (`rust:1.86-slim` → `debian:bookworm-slim`, ~93MB).

Push to `master` triggers the GitHub Actions CI pipeline; on success, Coolify
auto-deploys the container. Required secrets: `COOLIFY_API_TOKEN`,
`COOLIFY_APP_UUID`, `COOLIFY_BASE_URL`.

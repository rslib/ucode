# MCP Hooks

Model Context Protocol server lifecycle events.

## `mcp_server_connected`

- **Safety tier:** Safe
- **Hook category:** mcp
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-mcp/on-connected`

**Payload schema:**

```json
{
  "server_name": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `mcp_server_disconnected`

- **Safety tier:** Safe
- **Hook category:** mcp
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-mcp/on-disconnected`

**Payload schema:**

```json
{
  "server_name": "string",
  "reason": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `mcp_server_launch`

- **Safety tier:** Safe
- **Hook category:** mcp
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-mcp/on-launch`

**Payload schema:**

```json
{
  "server_name": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `mcp_server_restart`

- **Safety tier:** Safe
- **Hook category:** mcp
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-mcp/on-restart`

**Payload schema:**

```json
{
  "server_name": "string",
  "reason": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `mcp_server_crash`

- **Safety tier:** Safe
- **Hook category:** mcp
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-mcp/on-crash`

**Payload schema:**

```json
{
  "server_name": "string",
  "error": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `mcp_tool_invoked`

- **Safety tier:** Safe
- **Hook category:** mcp
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-mcp/on-tool-invoked`

**Payload schema:**

```json
{
  "server_name": "string",
  "tool_name": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

# Hook Payload Reference

Complete reference for all 67 hook events across 20 categories in the ucode plugin system.

## Categories

| Category | Events | Description |
|----------|--------|-------------|
| [session](session.md) | 5 | Session lifecycle events |
| [message](message.md) | 4 | Message flow events |
| [model](model.md) | 7 | Model selection and invocation events |
| [tool](tool.md) | 4 | Tool execution events |
| [tool_fs](tool_fs.md) | 4 | File system operation events |
| [tool_cmd](tool_cmd.md) | 2 | Command execution events |
| [tool_patch](tool_patch.md) | 2 | Patch application events |
| [context](context.md) | 4 | Token context management events |
| [agent](agent.md) | 5 | Agent lifecycle events |
| [approval](approval.md) | 5 | Access control and approval events |
| [auth](auth.md) | 3 | Authentication and provider events |
| [mcp](mcp.md) | 6 | Model Context Protocol server events |
| [skill](skill.md) | 2 | Skill activation events |
| [plugin](plugin.md) | 3 | Plugin lifecycle events |
| [checkpoint](checkpoint.md) | 2 | Session checkpoint events |
| [budget](budget.md) | 3 | Cost tracking and budget events |
| [job](job.md) | 1 | Background job lifecycle events |
| [command](command.md) | 2 | Command invocation events |
| [diagnostic](diagnostic.md) | 1 | Error and diagnostic events |
| [transform](transform.md) | 2 | Data transformation events |

## Safety Tiers

- **Safe** -- Pure observability. Plugin can only return `Ok`.
- **Guarded** -- Plugin may return `Modify` with bounded changes.
- **Risky** -- Plugin may return `Veto` to block the action. Requires user approval.

## Payload Versioning

All payloads start at version 1.0.0. Minor bumps for additive fields, major for breaking changes.

## Response Types

Each hook supports one or more response types based on its safety tier:

### Safe Hooks
```
Ok -- Hook was observed and processed without action
```

### Guarded Hooks
```
Ok -- Hook was observed without action
Modify { changes } -- Propose modifications to the hook payload
```

### Risky Hooks
```
Ok -- Hook was observed without action
Modify { changes } -- Propose modifications to the hook payload
Veto { reason } -- Block the action entirely (requires user approval)
```

## Hook Interface Format

All hooks use the WIT (WebAssembly Interface Types) interface format:

```
ucode:hooks-{category}/{event-name}
```

Example: `ucode:hooks-session/on-start`

## Event Count Summary

- **Total Events:** 67
- **Safe Tiers:** 44
- **Guarded Tiers:** 18
- **Risky Tiers:** 5

## Quick Navigation

- [Getting Started with Hooks](../getting-started.md)
- [Hook Integration Guide](../integration-guide.md)
- [Safety and Permissions](../safety-and-permissions.md)

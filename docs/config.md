# ucode Configuration

## Config File Location

ucode loads configuration from a TOML file at:

```
${UCODE_HOME}/ucode.toml
```

Where `UCODE_HOME` defaults to `${XDG_CONFIG_HOME:-~/.config}/ucode`.

Override the config directory by setting `UCODE_HOME`:

```bash
export UCODE_HOME=/path/to/custom/config
```

## Config File Format

The config file uses TOML format. The primary section is `[providers]`, which defines LLM provider connections.

### Example `ucode.toml`

```toml
# Provider configurations
# Each provider is a [providers.<name>] section.

[providers.anthropic]
type = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"

[providers.openai]
type = "openai"
api_key_env = "OPENAI_API_KEY"

[providers.gemini]
type = "gemini"
api_key_env = "GEMINI_API_KEY"

[providers.ollama]
type = "ollama"
# Ollama runs locally, no API key needed
# base_url defaults to http://localhost:11434

[providers.custom-openai]
type = "openai"
base_url = "https://api.together.xyz/v1"
api_key_env = "TOGETHER_API_KEY"
```

### Provider Configuration Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | string | yes | Adapter type: `"openai"`, `"anthropic"`, `"ollama"`, `"gemini"`, `"copilot"` |
| `api_key_env` | string | no | Environment variable name containing the API key |
| `base_url` | string | no | Override the default API endpoint |
| `headers` | table | no | Additional HTTP headers as key-value pairs |

### Default Base URLs

| Adapter | Default URL |
|---------|-------------|
| `openai` | `https://api.openai.com/v1` |
| `anthropic` | `https://api.anthropic.com/v1` |
| `ollama` | `http://localhost:11434` |
| `gemini` | `https://generativelanguage.googleapis.com` |
| `copilot` | `https://api.githubcopilot.com` |

### Custom Headers

```toml
[providers.custom]
type = "openai"
base_url = "https://my-proxy.example.com/v1"
api_key_env = "CUSTOM_API_KEY"

[providers.custom.headers]
X-Custom-Header = "value"
X-Organization = "my-org"
```

## Environment Variable Auto-Discovery

If no config file exists (or it has no `[providers]` section), ucode automatically discovers providers from well-known environment variables:

| Environment Variable | Provider Name | Adapter |
|---------------------|---------------|---------|
| `ANTHROPIC_API_KEY` | `anthropic` | Anthropic |
| `OPENAI_API_KEY` | `openai` | OpenAI |
| `GEMINI_API_KEY` | `gemini` | Gemini |
| `GOOGLE_API_KEY` | `gemini` | Gemini |

This means you can use ucode with zero configuration -- just set an API key:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
ucode  # launches TUI with Anthropic provider
```

## Precedence Rules

Configuration is resolved in this order (later overrides earlier):

1. **Built-in defaults** -- default base URLs, default models per adapter
2. **Config file** (`${UCODE_HOME}/ucode.toml`) -- explicit provider definitions
3. **Environment variable discovery** -- only adds providers NOT already defined in the config file

The config file always takes precedence over env-var discovery. If you define `[providers.openai]` in your config file with a custom `base_url`, setting `OPENAI_API_KEY` will NOT override that definition.

## Default Provider Selection

When multiple providers are configured, ucode selects the default in this order:

1. `anthropic` (if available)
2. `openai` (if available)
3. `gemini` (if available)
4. First available provider (alphabetical)

## Default Models

When no model is explicitly specified, ucode uses these defaults:

| Adapter | Default Model |
|---------|---------------|
| Anthropic | `claude-sonnet-4-20250514` |
| OpenAI | `gpt-4o` |
| Gemini | `gemini-2.0-flash` |
| Ollama | `llama3.2` |
| Copilot | `gpt-4o` |

## Integration Testing

To test with a custom config directory:

```bash
export UCODE_HOME=/tmp/ucode-test
mkdir -p "$UCODE_HOME"
cat > "$UCODE_HOME/ucode.toml" << 'EOF'
[providers.test]
type = "ollama"
EOF
ucode
```

## Credential Storage

API keys referenced by `api_key_env` are read from environment variables at runtime. For persistent credential storage, use the keyring-based auth system:

```bash
ucode auth set-key openai    # stores in OS keychain
ucode auth status             # shows credential status
ucode auth login anthropic    # browser-based OAuth
```

See the `/connect` command in the TUI for interactive credential management.

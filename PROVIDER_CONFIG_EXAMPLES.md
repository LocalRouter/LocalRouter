# Provider Configuration Examples

This document shows how to configure different providers using the flexible `provider_config` field in your `settings.yaml`.

## Overview

Each provider can have its own custom configuration structure. The `provider_config` field accepts a JSON/YAML object with provider-specific settings.

**Important:** API keys are **NOT** stored in this file. They are stored securely in your system keyring (macOS Keychain, Windows Credential Manager, Linux Secret Service). Use the `api_key_ref` field to reference them.

## Configuration File Location

- **macOS**: `~/Library/Application Support/LocalRouter/settings.yaml`
- **Linux**: `~/.localrouter/settings.yaml`
- **Windows**: `%APPDATA%\LocalRouter\settings.yaml`

---

## Example: Complete Configuration

```yaml
version: 1

server:
  host: "127.0.0.1"
  port: 3000
  enable_cors: true

providers:
  # Local Ollama - No API key needed
  - name: "ollama"
    provider_type: "ollama"
    enabled: true
    provider_config:
      base_url: "http://localhost:11434"
      timeout_seconds: 120

  # OpenAI - Default endpoint
  - name: "openai"
    provider_type: "openai"
    enabled: true
    api_key_ref: "openai"  # References keyring entry
    provider_config:
      timeout_seconds: 30
      organization: "org-xyz123"  # Optional organization ID

  # OpenAI - Custom endpoint (e.g., proxy or self-hosted)
  - name: "openai-proxy"
    provider_type: "openai"
    enabled: true
    api_key_ref: "openai-proxy"
    provider_config:
      endpoint: "https://my-proxy.example.com/v1"
      timeout_seconds: 60

  # Anthropic - Standard configuration
  - name: "anthropic"
    provider_type: "anthropic"
    enabled: true
    api_key_ref: "anthropic"
    provider_config:
      version: "2023-06-01"  # API version
      timeout_seconds: 120

  # Google Gemini - Custom base URL
  - name: "gemini"
    provider_type: "gemini"
    enabled: true
    api_key_ref: "gemini"
    provider_config:
      base_url: "https://generativelanguage.googleapis.com/v1beta"

  # OpenRouter - With custom headers
  - name: "openrouter"
    provider_type: "openrouter"
    enabled: true
    api_key_ref: "openrouter"
    provider_config:
      app_name: "My Application"
      app_url: "https://myapp.com"
      extra_headers:
        X-Custom-Header: "value"
        X-Title: "My App"

  # Custom provider
  - name: "my-custom-llm"
    provider_type: "custom"
    enabled: false
    api_key_ref: "custom-llm"
    provider_config:
      endpoint: "https://api.mycustomllm.com/v1"
      timeout_seconds: 45
      custom_setting: "value"
      nested_config:
        setting1: "value1"
        setting2: "value2"

routers:
  - name: "Minimum Cost"
    model_selection:
      type: "automatic"
      providers: []
    strategies:
      - "local_first"
      - "lowest_cost"
    fallback_enabled: true
    rate_limiters: []

  - name: "Maximum Performance"
    model_selection:
      type: "automatic"
      providers: []
    strategies:
      - "highest_performance"
    fallback_enabled: true
    rate_limiters: []

logging:
  level: "info"
  enable_access_log: true
  retention_days: 30
```

---

## Provider-Specific Configuration Details

### Ollama

**No API key required** - Local models only

```yaml
- name: "ollama"
  provider_type: "ollama"
  enabled: true
  provider_config:
    base_url: "http://localhost:11434"  # Default: http://localhost:11434
    timeout_seconds: 120                 # Optional: request timeout
```

**Supported fields:**
- `base_url` (string) - Ollama server URL
- `timeout_seconds` (number) - HTTP request timeout

---

### OpenAI

**Requires API key** stored in keyring

```yaml
- name: "openai"
  provider_type: "openai"
  enabled: true
  api_key_ref: "openai"
  provider_config:
    endpoint: "https://api.openai.com/v1"  # Optional: custom endpoint
    organization: "org-xyz"                 # Optional: organization ID
    timeout_seconds: 30                     # Optional: request timeout
```

**Supported fields:**
- `endpoint` (string) - Custom API endpoint (for proxies or Azure)
- `organization` (string) - OpenAI organization ID
- `timeout_seconds` (number) - HTTP request timeout

**Example with Azure OpenAI:**
```yaml
- name: "azure-openai"
  provider_type: "openai"
  enabled: true
  api_key_ref: "azure-openai"
  provider_config:
    endpoint: "https://YOUR-RESOURCE.openai.azure.com/openai/deployments/YOUR-DEPLOYMENT"
    timeout_seconds: 60
```

---

### Anthropic (Claude)

**Requires API key** stored in keyring

```yaml
- name: "anthropic"
  provider_type: "anthropic"
  enabled: true
  api_key_ref: "anthropic"
  provider_config:
    endpoint: "https://api.anthropic.com/v1"  # Optional: custom endpoint
    version: "2023-06-01"                      # Optional: API version
    timeout_seconds: 120                       # Optional: request timeout
```

**Supported fields:**
- `endpoint` (string) - Custom API endpoint
- `version` (string) - Anthropic API version header
- `timeout_seconds` (number) - HTTP request timeout

---

### Google Gemini

**Requires API key** stored in keyring

```yaml
- name: "gemini"
  provider_type: "gemini"
  enabled: true
  api_key_ref: "gemini"
  provider_config:
    base_url: "https://generativelanguage.googleapis.com/v1beta"  # Optional
```

**Supported fields:**
- `base_url` (string) - Gemini API base URL

---

### OpenRouter

**Requires API key** stored in keyring

```yaml
- name: "openrouter"
  provider_type: "openrouter"
  enabled: true
  api_key_ref: "openrouter"
  provider_config:
    app_name: "My Application"    # Optional: for routing headers
    app_url: "https://myapp.com"  # Optional: for routing headers
    extra_headers:                # Optional: custom headers
      X-Custom: "value"
```

**Supported fields:**
- `app_name` (string) - Application name for OpenRouter's routing
- `app_url` (string) - Application URL for OpenRouter's routing
- `extra_headers` (object) - Additional HTTP headers to send

---

### Custom Provider

For providers not built into LocalRouter:

```yaml
- name: "my-provider"
  provider_type: "custom"
  enabled: true
  api_key_ref: "my-provider"
  provider_config:
    endpoint: "https://api.example.com/v1"  # Required
    timeout_seconds: 30
    # Any additional fields your provider needs
    custom_field: "value"
```

---

## Managing API Keys

API keys are stored in your system keyring, NOT in the configuration file.

### Store an API key:

```bash
# Via Tauri command (from the UI)
set_provider_api_key("openai", "sk-proj-...")
```

### Check if a key exists:

```bash
# Via Tauri command (from the UI)
has_provider_api_key("openai")  # Returns true/false
```

### Delete an API key:

```bash
# Via Tauri command (from the UI)
delete_provider_api_key("openai")
```

### Where keys are stored:

- **macOS**: Keychain Access.app → Search "LocalRouter-Providers"
- **Windows**: Credential Manager → "LocalRouter-Providers"
- **Linux**: Use `secret-tool` command:
  ```bash
  secret-tool search service LocalRouter-Providers
  ```

---

## Tips

1. **Multiple providers of same type**: Use different `name` values
   ```yaml
   - name: "openai-primary"
     provider_type: "openai"
     api_key_ref: "openai-1"

   - name: "openai-backup"
     provider_type: "openai"
     api_key_ref: "openai-2"
   ```

2. **Disable providers temporarily**: Set `enabled: false`

3. **Version control**: You can safely commit `settings.yaml` - no secrets!

4. **Testing**: Use `enabled: false` for production keys during development

5. **Custom fields**: The `provider_config` accepts any valid JSON/YAML structure

---

## Validation

The configuration is validated on load:
- Provider names must not be empty
- `provider_config` must be a JSON object (not a primitive)
- Providers validate their own config fields

Invalid configurations will show helpful error messages on startup.

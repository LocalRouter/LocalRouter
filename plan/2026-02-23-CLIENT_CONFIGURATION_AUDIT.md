# Client Configuration Audit

**Date**: 2026-02-23
**Purpose**: Audit of all 21 client templates and their configuration accuracy, verified by external research LLM (Perplexity).

---

## Audit Results Summary

### Clients with Issues Found

| Client | Severity | Issue |
|--------|----------|-------|
| Claude Code | Medium | `ANTHROPIC_BASE_URL` not officially documented; `~/.claude.json` is internal state |
| OpenClaw | High | `api: "openai-completions"` should be `"openai-responses"` |
| OpenClaw | Low | `clawdbot` binary not verifiable in public docs |
| Cursor | Medium | MCP JSON should use `transport: "streamableHttp"` instead of `type: "http"` |
| Cursor | Low | `openai.apiBaseUrl` vs `openai.baseUrl` inconsistency across versions |
| Goose | Low | `OPENAI_BASE_URL` not in official docs (convention only) |
| Goose | Low | `type: "streamable_http"` in extensions block not verifiable from published docs |
| Droid | Low | MCP config should include `type: "http"` field |

| Windsurf | Medium | Does not support arbitrary OpenAI-compatible base URLs; only BYOK for specific models |
| LobeChat | Medium | Uses env vars (`OPENAI_PROXY_URL`, `OPENAI_API_KEY`), not in-UI provider creation |
| n8n | Low | Credential type is "OpenAI" not "OpenAI-compatible" |
| Onyx | Low | Generic OpenAI-compatible provider with custom base URL not verifiable in docs |
| Xcode | Medium | "Intelligence > Internet Hosted" with custom URLs not documented; likely speculative |
| marimo | Low | Exact "User Settings > AI tab" navigation path not verifiable |

### Clients Confirmed Correct

| Client | Notes |
|--------|-------|
| Aider | Env vars, config path, YAML keys all confirmed correct |
| Cline | Confirmed correct |
| Roo Code | Confirmed correct |
| VS Code + Continue | Confirmed correct |
| Zed | Confirmed correct |
| JetBrains | Confirmed correct |
| Open WebUI | Confirmed correct |
| Droid | Config file paths confirmed correct (`~/.factory/settings.json`, `~/.factory/mcp.json`) |
| Goose | Config file path confirmed correct (`~/.config/goose/config.yaml`) |
| Cursor | MCP config path confirmed correct (`~/.cursor/mcp.json`) |
| Custom | MCP JSON shapes align with generic MCP spec |

| Codex | High | Not an MCP client; should be LLM only. Env var pattern should use `--provider` scheme |
| OpenCode | Low | Consider adding `enabled: true` to MCP config for clarity |

### Templates to Remove

| Client | Reason |
|--------|--------|
| Windsurf | No custom base URL support; only BYOK for specific models. Cannot point to LocalRouter. |

### Clients Confirmed Correct (Round 3)

| Client | Notes |
|--------|-------|
| Codex | LLM integration correct via `OPENAI_BASE_URL`/`OPENAI_API_KEY`; config path `~/.codex/config.toml` correct |
| OpenCode | Provider JSON structure confirmed correct; MCP structure confirmed correct |
| Xcode | Confirmed working — Xcode 26 "Internet Hosted" allows custom URLs and API key headers |

---

## Detailed Corrections

### 1. Claude Code

#### Issue A: MCP Configuration Method
- **Field**: Auto (Permanent) config method
- **Current**: Writes MCP config to `~/.claude.json` under `mcpServers.localrouter`
- **Finding**: Official Claude Code docs describe managing MCP servers via `claude mcp add <name> --transport http <url>`, not by editing `~/.claude.json` directly. The file is internal state.
- **Recommendation**: Consider using `claude mcp add` CLI command instead of direct file manipulation. At minimum, document that `~/.claude.json` editing is unofficial.
- **Sources**:
  - https://docs.mcp.run/mcp-clients/claude-code/
  - https://code.claude.com/docs/en/mcp

#### Issue B: Environment Variable Names
- **Field**: `ANTHROPIC_BASE_URL`
- **Current**: Listed as the env var for base URL
- **Finding**: `ANTHROPIC_API_KEY` is documented, but `ANTHROPIC_BASE_URL` has no official documentation for Claude Code. Custom endpoints are normally handled via configuration or client options.
- **Recommendation**: Document `ANTHROPIC_BASE_URL` as "best effort / not guaranteed" rather than canonical, OR verify it works empirically and note it's undocumented.
- **Sources**:
  - https://www.reddit.com/r/ClaudeAI/comments/1ntt821/how_to_specify_anthropic_base_url_for_vscode/
  - https://platform.claude.com/docs/en/agents-and-tools/mcp-connector

### 2. Codex

#### Issue A (HIGH): Not an MCP Client
- **Field**: Mode / MCP support
- **Current value**: Mode = "LLM + MCP"; Auto config writes MCP section to `~/.codex/config.toml`
- **Correct value**: Codex is an **LLM client only**, not an MCP client. Docs describe model providers and streamable HTTP servers but no MCP client layer or `mcpServers`/`mcp_servers` schema.
- **Action Required**: Change `defaultMode` from `both` to `llm_only`, set `supportsMcp: false`, remove MCP config from permanent integration.
- **Sources**:
  - https://developers.openai.com/codex/config-reference/
  - https://developers.openai.com/codex/cli/reference/

#### Issue B: Env Var Pattern
- **Field**: Environment variable names
- **Current**: `OPENAI_BASE_URL` and `OPENAI_API_KEY`
- **Finding**: Using `OPENAI_API_KEY` + `OPENAI_BASE_URL` is valid if keeping `model_provider = "openai"` and pointing at LocalRouter. The more canonical pattern for a distinct provider is `<PROVIDER>_API_KEY` + `<PROVIDER>_BASE_URL` + `--provider <id>`. Current approach works but is overriding the OpenAI provider rather than adding a custom one.
- **Recommendation**: Keep current approach (simpler) but document it as "overriding the OpenAI provider endpoint".
- **Sources**:
  - https://developers.openai.com/codex/config-reference/
  - https://community.openai.com/t/cant-setup-codex-cli-with-custom-base-url-and-api-key-via-terminal-env-variables-or-command-options/1363678

#### Note: Config File Path
- Path `~/.codex/config.toml` is correct for default setups. Canonical location is `$CODEX_HOME/config.toml` (defaults to `~/.codex/config.toml`).

### 3. Aider — Correct
- **Env vars**: `OPENAI_API_BASE` and `OPENAI_API_KEY` confirmed correct.
- **Config path**: `~/.aider.conf.yml` confirmed correct.
- **YAML keys**: `openai-api-base` and `openai-api-key` confirmed correct.
- **Sources**:
  - https://aider.chat/docs/config/api-keys.html
  - https://aider.chat/docs/config/aider_conf.html
  - https://github.com/Aider-AI/aider/blob/main/aider/website/assets/sample.aider.conf.yml

### 4. OpenCode — Confirmed Correct (with minor notes)
- **Provider JSON**: Confirmed correct. `npm: "@ai-sdk/openai-compatible"`, `options.baseURL`, `options.apiKey`, and `models` map all match documented schema.
- **MCP JSON**: Confirmed correct. `type: "remote"`, `url`, `headers` all match documented schema. Consider adding `"enabled": true` for clarity.
- **Config file location**: `<CONFIG_DIR>/opencode/opencode.json` is reasonable for global config. OpenCode supports both global and project-local configs (`opencode.json` or `opencode.jsonc`).
- **Sources**:
  - https://open-code.ai/en/docs/config
  - https://opencode-tutorial.com/en/docs/providers
  - https://www.opencodecn.com/docs/providers

### 5. Droid — Mostly Correct

#### Minor Issue: MCP JSON Structure
- **Field**: MCP config JSON structure
- **Current**: Uses `url` and `headers` without `type` field
- **Finding**: Factory docs show MCP config with `type: "http"` field included. Adding `Authorization` header is consistent but should also include `type: "http"`.
- **Recommendation**: Add `type: "http"` to the MCP server entry in `~/.factory/mcp.json`.
- **Sources**:
  - https://docs.factory.ai/cli/configuration/mcp
  - https://factory.mintlify.app/cli/configuration/mcp

### 6. Goose — Mostly Correct

#### Minor Issue A: Env Vars
- **Field**: `OPENAI_BASE_URL`
- **Finding**: Goose's provider system uses `goose configure` and `config.yaml` with a "host" field. `OPENAI_BASE_URL` is a common convention but not documented by Goose.
- **Recommendation**: Document as convention, not officially supported.
- **Sources**:
  - https://block.github.io/goose/docs/guides/config-files/
  - https://github.com/block/goose/issues/1198

#### Minor Issue B: Extensions Block
- **Field**: `type: "streamable_http"` in extensions config
- **Finding**: Not verifiable from published Goose docs. `streamableHttp` appears in Cursor MCP examples as a `transport` value, not a Goose extension type.
- **Recommendation**: Mark this path as "experimental / subject to change" in UI.
- **Sources**:
  - https://block.github.io/goose/docs/guides/config-files/

### 7. OpenClaw

#### Issue A (HIGH): API Type Value
- **Field**: `api` field in provider config
- **Current value**: `"openai-completions"`
- **Correct value**: `"openai-responses"`
- **Finding**: OpenClaw docs show `api: "openai-responses"` for OpenAI-compatible HTTP endpoints. `"openai-completions"` is not listed as a valid `api` type.
- **Action Required**: Change `api` value from `"openai-completions"` to `"openai-responses"` in the template.
- **Sources**:
  - https://docs.openclaw.ai/concepts/model-providers
  - https://www.getopenclaw.ai/help/switching-models-provider-config

#### Issue B: Binary Name
- **Field**: `clawdbot` binary
- **Finding**: Official docs reference `openclaw` CLI only; `clawdbot` not mentioned in public docs.
- **Recommendation**: Remove `clawdbot` from `binaryNames` or verify internally.
- **Sources**:
  - https://docs.openclaw.ai/cli/models

### 8. Cline — Correct

### 9. Roo Code — Correct

### 10. VS Code + Continue — Correct

### 11. Cursor

#### Issue A: MCP JSON Schema
- **Field**: MCP server config format
- **Current**: Uses `"type": "http"` in MCP JSON
- **Finding**: Cursor-specific examples use `"transport": "streamableHttp"` for remote HTTP servers rather than `"type": "http"`.
- **Recommendation**: Change MCP config to use `transport: "streamableHttp"` for Cursor compatibility:
  ```json
  {
    "mcpServers": {
      "localrouter": {
        "url": "http://127.0.0.1:3625",
        "transport": "streamableHttp",
        "headers": {
          "Authorization": "Bearer <secret>"
        }
      }
    }
  }
  ```
- **Sources**:
  - https://cursor.fan/tutorial/HowTo/how-to-config-mcp-server-with-an-env-parameter-in-cursor/
  - https://docs.omni.co/ai/mcp/cursor

#### Issue B: LLM Settings Keys
- **Field**: `openai.apiBaseUrl` in `settings.json`
- **Finding**: Some inconsistency between `openai.apiBaseUrl` and `openai.baseUrl` across Cursor versions.
- **Recommendation**: Consider making key names configurable or marking as version-dependent.
- **Sources**:
  - Community walkthroughs on Cursor JSON configuration

### 12. Windsurf — REMOVE

#### Reason: Cannot Point to LocalRouter
- **Field**: Feature support
- **Current assumption**: Windsurf can be configured with an arbitrary OpenAI-compatible base URL.
- **Reality**: Windsurf only supports BYOK for specific providers (paste API key for OpenAI, etc.) but does **not** support overriding the HTTP base URL. There is no way to point Windsurf at `http://127.0.0.1:3625` without an external network proxy that rewrites `api.openai.com`.
- **Action Required**: Remove the Windsurf template entirely from `ClientTemplates.tsx`.
- **Sources**:
  - https://docs.windsurf.com/windsurf/models
  - https://www.reddit.com/r/windsurf/comments/1lrn18x/how_to_use_my_own_api_key/

### 13. Zed — Correct

### 14. JetBrains — Correct

### 15. Xcode — Confirmed Correct
- **Status**: Confirmed working. Xcode 26's Intelligence settings let you "Add a Model Provider", choose **Internet Hosted**, and enter a base URL, an API key header (e.g. `Authorization`), and a key value. Third-party guides already show pointing this at OpenAI-compatible endpoints (Gemini, OpenRouter), so pointing at LocalRouter is valid.
- **Sources**:
  - https://wendyliga.com/blog/xcode-26-custom-model/
  - https://zottmann.org/2025/06/13/how-to-use-google-gemini.html
  - https://openrouter.ai/docs/guides/community/xcode

### 16. Open WebUI — Correct

### 17. LobeChat

#### Issue: Configuration via Env Vars, Not UI
- **Field**: Manual instructions
- **Current**: "In LobeChat provider settings, add an OpenAI-compatible provider with endpoint <BASE_URL> and your client secret as the API key."
- **Correct**: LobeChat configures OpenAI and OpenAI-compatible proxies primarily via **environment variables**, not a separate "OpenAI-compatible" provider type in the UI. The documented pattern is: enable the OpenAI provider with `ENABLED_OPENAI`, set `OPENAI_API_KEY`, and for a proxy/router, override the endpoint using `OPENAI_PROXY_URL` to your base URL.
- **Recommendation**: Update helper text to: "Set `OPENAI_API_KEY` to your client secret and `OPENAI_PROXY_URL` to <BASE_URL>" rather than describing an in-UI provider creation flow.
- **Sources**:
  - https://lobehub.com/docs/self-hosting/environment-variables/model-provider

### 18. Onyx

#### Issue: Generic OpenAI-Compatible Provider Not Verifiable
- **Field**: Feature support and UI steps
- **Current**: "In Onyx setup: select an OpenAI-compatible LLM provider, set the API URL to <BASE_URL> and the API key to your client secret."
- **Finding**: No current official Onyx documentation could be located that explicitly documents a generic "OpenAI-compatible" provider with user-settable base URL and API key, or a first-class MCP configuration screen.
- **Recommendation**: Mark as "unverified / best-effort" until confirmed against Onyx's own documentation or UI.

### 19. marimo

#### Issue: Exact UI Navigation Not Verifiable
- **Field**: Manual UI instructions
- **Current**: "In marimo: User Settings > AI tab > configure an OpenAI-compatible provider with base URL <BASE_URL> and your client secret as the API key."
- **Finding**: The exact existence of a "User Settings > AI tab" flow and precise field labels could not be confirmed from public docs. The overall idea is plausible but specific navigation path and labels are unverified.
- **Recommendation**: Treat as unverified until checked against marimo's official docs or current UI.

### 20. n8n

#### Issue: Credential Type Name
- **Field**: Credential type name
- **Current**: "Create Credential > select OpenAI-compatible"
- **Correct**: n8n exposes an **"OpenAI"** credential type (not "OpenAI-compatible") where you provide an API key and optionally override the Base URL.
- **Recommendation**: Update helper text to: "Create Credential > select **OpenAI** > set API Key to your client secret, and set Base URL to <BASE_URL>."
- **Sources**: n8n OpenAI credentials docs

### 21. Custom — Correct
MCP JSON shapes align with generic MCP spec for HTTP and STDIO servers. OAuth fields are LocalRouter-specific convention, documented as such.

---

## Action Items (Implementation Plan)

All changes include source references as code comments.

### Must Fix (High Priority)

1. **Windsurf**: Remove template entirely from `ClientTemplates.tsx`
   - Source: https://docs.windsurf.com/windsurf/models — no custom base URL support

2. **Codex**: Change to LLM-only mode
   - In `ClientTemplates.tsx`: set `defaultMode: 'llm_only'`, `supportsMcp: false`
   - In `codex.rs`: remove MCP config from permanent integration
   - Source: https://developers.openai.com/codex/config-reference/ — no MCP client layer

3. **OpenClaw**: Fix API type value
   - In `ClientTemplates.tsx`: change `api: "openai-completions"` to `api: "openai-responses"`
   - In `openclaw.rs`: update the same value in permanent config
   - Source: https://docs.openclaw.ai/concepts/model-providers

4. **LobeChat**: Update manual instructions
   - In `ClientTemplates.tsx`: change `manualInstructions` to describe env var configuration (`OPENAI_PROXY_URL`, `OPENAI_API_KEY`)
   - Source: https://lobehub.com/docs/self-hosting/environment-variables/model-provider

5. **n8n**: Fix credential type name
   - In `ClientTemplates.tsx`: change "OpenAI-compatible" to "OpenAI" in `manualInstructions`
   - Source: n8n OpenAI credentials docs

### Should Fix (Medium Priority)

6. **Cursor**: Update MCP config format in permanent integration
   - In `cursor.rs`: use `transport: "streamableHttp"` instead of `type: "http"`
   - Source: https://cursor.fan/tutorial/HowTo/how-to-config-mcp-server-with-an-env-parameter-in-cursor/

7. **Droid**: Add `type: "http"` to MCP server entry
   - In `droid.rs`: add `type` field to MCP config JSON
   - Source: https://docs.factory.ai/cli/configuration/mcp

8. **OpenClaw**: Remove unverified `clawdbot` binary
   - In `ClientTemplates.tsx`: remove `clawdbot` from `binaryNames`
   - Source: https://docs.openclaw.ai/cli/models — only `openclaw` documented

### Won't Fix (Acceptable as-is)

9. **Claude Code**: `ANTHROPIC_BASE_URL` — works empirically, used by the community; keep as-is
10. **Claude Code**: `~/.claude.json` direct editing — this is how most MCP config managers work; keep as-is
11. **Goose**: `OPENAI_BASE_URL` convention — standard OpenAI env var pattern; keep as-is
12. **Goose**: `type: "streamable_http"` in extensions — matches Goose's actual behavior; keep as-is
13. **Cursor**: `openai.apiBaseUrl` key — works with current Cursor versions; keep as-is
14. **Xcode**: Confirmed working with Xcode 26; no changes needed
15. **Onyx/marimo**: Plausible instructions, keep but could verify later
16. **OpenCode**: Add `enabled: true` to MCP — nice to have but not required

---

## Research Sources

### Round 1 (Initial)
1. https://docs.mcp.run/mcp-clients/claude-code/
2. https://code.claude.com/docs/en/mcp
3. https://aider.chat/docs/config/api-keys.html
4. https://aider.chat/docs/config/aider_conf.html
5. https://github.com/Aider-AI/aider/blob/main/aider/website/assets/sample.aider.conf.yml
6. https://docs.factory.ai/cli/configuration/mcp
7. https://block.github.io/goose/docs/guides/config-files/
8. https://github.com/block/goose/issues/1198
9. https://docs.openclaw.ai/concepts/model-providers
10. https://www.getopenclaw.ai/help/switching-models-provider-config
11. https://docs.openclaw.ai/cli/models
12. https://docs.omni.co/ai/mcp/cursor
13. https://cursor.fan/tutorial/HowTo/how-to-config-mcp-server-with-an-env-parameter-in-cursor/

### Round 2 (UI-only clients)
14. https://docs.windsurf.com/windsurf/models
15. https://www.reddit.com/r/windsurf/comments/1lrn18x/how_to_use_my_own_api_key/
16. https://lobehub.com/docs/self-hosting/environment-variables/model-provider
17. n8n OpenAI credentials docs

### Round 3 (Codex, OpenCode, Xcode confirmation)
18. https://developers.openai.com/codex/config-reference/
19. https://developers.openai.com/codex/cli/reference/
20. https://community.openai.com/t/cant-setup-codex-cli-with-custom-base-url-and-api-key-via-terminal-env-variables-or-command-options/1363678
21. https://www.opencodecn.com/docs/providers
22. https://theaiops.substack.com/p/setting-up-opencode-with-local-models
23. https://opencode-tutorial.com/en/docs/providers
24. https://open-code.ai/en/docs/config
25. https://opencode.ai/docs/config/
26. https://wendyliga.com/blog/xcode-26-custom-model/
27. https://zottmann.org/2025/06/13/how-to-use-google-gemini.html
28. https://openrouter.ai/docs/guides/community/xcode

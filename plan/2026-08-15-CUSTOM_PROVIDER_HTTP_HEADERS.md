# Custom LLM Provider HTTP Headers (GitHub issue #12)

Allow users to define custom HTTP headers on the generic OpenAI-compatible
("Custom") provider. Custom endpoints (corporate gateways, vLLM behind
proxies, Azure-style deployments, Cloudflare AI Gateway, …) often require
extra headers beyond `Authorization`.

## Design

- New config key `custom_headers` on the `openai_compatible` provider type.
- Value is a single string, **one header per line**, `Name: Value` format.
  Keeping it a plain `String` lets it flow through the existing
  `HashMap<String, String>` config pipeline (wizard → Tauri command →
  registry → factory) unchanged.
- **Stored in the system keyring, never in the config file.** Custom headers
  routinely carry credentials (Azure-style `api-key:`, gateway tokens), and
  the codebase already treats header-borne secrets that way — MCP servers
  keep auth header values in the keyring (`McpAuthConfig::CustomHeaders`
  stores refs only), and provider API keys are stripped from
  `provider_config` before save. Account name:
  `"{instance_name}:custom_headers"` under the existing
  `LocalRouter-Providers` service.
- Headers are applied to **all** outgoing requests of the provider
  (health check, list models, chat completions, streaming, embeddings).
- Applied via `RequestBuilder::headers`, which *replaces* same-named entries,
  so a user-supplied `Authorization`/`Content-Type` overrides the default
  rather than being sent twice. Repeating a name across lines emits the
  header multiple times, as HTTP allows.
- New `ParameterType::Headers` variant so the UI renders a multi-line
  textarea instead of a single-line input.

### Keychain lifecycle

Custom headers follow the API key through every provider lifecycle path:
create, update (clearing the field clears the entry), rename (migrated to the
new account name), clone (copied), remove (deleted), and startup load
(re-injected into the provider config map).

### Known limitation

`lr_guardrails::ProviderInfo` (safety-model calls) carries only
`base_url`/`api_key` and builds its own HTTP requests, so guardrail calls to a
header-authenticated custom provider won't include these headers. Out of
scope here; the main serving path is fully covered.

## Task checklist

- [x] `crates/lr-providers/src/openai_compatible.rs`
  - [x] `extra_headers: HeaderMap` field + `with_extra_headers()`
  - [x] `parse_custom_headers()` — parse/validate `Name: Value` lines
        (validated with `reqwest::header::HeaderName/HeaderValue`)
  - [x] `apply_headers()` helper replacing the 5 inline auth-header blocks
  - [x] Unit tests: parsing (valid, empty lines, invalid), auth override
- [x] `crates/lr-providers/src/factory.rs`
  - [x] `ParameterType::Headers` variant (serializes as `"headers"`)
  - [x] `OpenAICompatibleProviderFactory::setup_parameters()` — optional
        `custom_headers` param
  - [x] `create()` parses `custom_headers`, `validate_config()` rejects bad ones
  - [x] Factory tests
- [x] Keychain storage
  - [x] `key_storage::custom_headers_account()` — account naming convention
  - [x] `commands_providers.rs` — `persist_secret_config_values()`,
        `migrate_secret()`, `delete_provider_secrets()` covering create,
        update, rename, clone, remove
  - [x] `main.rs` — re-inject headers from keyring on startup load
- [x] Frontend
  - [x] `src/components/ProviderForm.tsx` — render `headers` param as textarea
        (create flow)
  - [x] `src/views/resources/providers-panel.tsx` — same for the settings/edit
        tab, committed on blur (per-keystroke autosave would reject
        half-typed lines)
  - [x] `src/types/tauri-commands.ts` — sync stale `SetupParameter` type
- [x] `website/src/components/demo/mockData.ts` — add `openai_compatible`
      provider type with `custom_headers` so the demo showcases it
- [x] Verify: clippy + fmt + targeted tests + `npx tsc --noEmit`

## Final steps (mandatory)

1. [x] Plan review — re-check implementation against this plan
2. [x] Test coverage review — all new paths covered (16 new Rust tests)
3. [x] Bug hunt — fresh-eyes pass over the diff (fixed: duplicate-Authorization
       precedence, empty header name rejection, whitespace-only value allowed)
4. [x] Commit (no push)

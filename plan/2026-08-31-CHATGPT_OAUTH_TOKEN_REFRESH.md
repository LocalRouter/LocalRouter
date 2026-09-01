# ChatGPT Plus/Pro OAuth: live token resolution + automatic refresh

**Date**: 2026-08-31
**Status**: implemented

## Problem

A ChatGPT Plus/Pro (`openai-chatgpt-plus`) provider stops authenticating and
stays broken even after the user clicks **Reconnect**:

- `/v1/chat/completions` (translated to `/responses`) → upstream 401
- `/v1/responses` → `Responses API error 502: {"error":{"message":"Router
  error: Authentication failed","type":"provider_error"}}`

The only recovery is deleting and re-creating the provider instance.

## Root cause

1. `OpenAIProvider::from_oauth_or_key` (`crates/lr-providers/src/openai.rs:84`)
   reads `openai-codex_access_token` from the keychain **once**, at
   construction, into a plain `api_key: String`. `ProviderRegistry` caches that
   instance for the process lifetime, so the token is a snapshot taken at app
   boot.
2. Nothing ever refreshes the token. `OAuthManager::get_valid_credentials`
   (the only code path that calls `refresh_tokens`) has no callers — dead code.
   There is no background refresher.
3. Reconnect writes the new tokens to the keychain and
   `oauth_credentials.json`, but never rebuilds the registry's provider, so the
   live instance keeps sending the dead token.
4. `TokenExchanger` never honors the `_use_json_body` marker that
   `openai_codex.rs` sets, so the refresh request would POST it as a literal
   form field instead of using the JSON body OpenAI's token endpoint expects.
5. The keychain stores no expiry, so nothing can tell a stale token from a
   good one without calling upstream.

## Fix

**Resolve the token per request, refresh it automatically, and re-read the
keychain when the tokens change.**

1. `lr-oauth` `TokenExchanger`
   - Honor `_use_json_body`: strip the marker and send a JSON body (this is
     what `auth.openai.com/oauth/token` expects for refresh, matching codex-rs).
   - Persist `{account}_expires_at` (unix seconds) alongside the tokens so
     expiry survives restarts and is available without a JWT.

2. New `crates/lr-providers/src/oauth/token_source.rs`
   - `OAuthTokenSource`: process-wide, per-`provider_id` singleton that owns
     the current access token.
   - `access_token()`: serve the in-memory token while valid; otherwise
     re-read the keychain, and exchange the refresh token when the stored one
     is expired (or within the skew window).
   - `refresh_after_unauthorized(rejected)`: called on a real 401. If the
     keychain already holds a different token (a reconnect happened), adopt it;
     otherwise force a refresh-token exchange.
   - Single-flight via an async mutex so concurrent requests refresh once.
   - Mirrors refreshed credentials back into `OAuthStorage` so the settings UI
     shows the live token/expiry.
   - `notify_tokens_updated(provider_id)` drops the cached token — called from
     `OAuthManager::poll_oauth` (reconnect) and `delete_credentials`.

3. `OpenAIProvider`
   - `api_key: String` → `auth: ProviderAuth` (`ApiKey` | `OAuth(Arc<…>)`).
   - `auth_header()` becomes async and resolves through the token source.
   - ChatGPT-backend request paths retry once on `AppError::Unauthorized` after
     forcing a refresh.

Anthropic Claude subscription and GitHub Copilot snapshot their tokens the same
way; `OAuthTokenSource` is written to be reusable for them, but wiring them up
is out of scope here.

## Steps

1. `TokenExchanger`: JSON body + `expires_at` persistence (+ tests) ✅
2. `OAuthTokenSource` + global registry/invalidation (+ tests) ✅
3. `openai_codex.rs`: single shared `OAuthFlowConfig` ✅
4. `OpenAIProvider`: async auth resolution + 401 retry ✅
5. `OAuthManager`: invalidate on reconnect/delete ✅
6. Plan review, test coverage review, bug hunt, commit ✅

## Notes from the review pass

- A failed refresh is remembered for 5 minutes (`REFRESH_RETRY_BACKOFF_SECS`).
  Periodic health checks resolve the token, so a revoked grant would otherwise
  hit OpenAI's token endpoint on every health cycle. A reconnect clears it.
- The ChatGPT health check now resolves the token instead of reporting a flat
  "healthy", so an expired session shows up in the UI as unhealthy with the
  reconnect message rather than only failing at request time.
- `notify_tokens_updated` is the fast path for a reconnect; the 401 handler is
  the safety net when the UI never polls the flow to completion (the tokens
  still land in the keychain, and the next 401 adopts them).

# Fix Custom Provider Creation (#17)

## Problem

User reports inability to add DeepSeek (or GPT4All / llama.cpp) via the Custom
OpenAI-Compatible provider flow. Submission fails.

## Root Cause

`src/views/resources/providers-panel.tsx`:
- Custom tab renders `<ProviderForm providerType={genericType} onSubmit={handleCreateProvider}>`
  with the correct generic `ProviderType` passed **as a prop** (line 1796-1807).
- `handleCreateProvider(instanceName, config)` (line 393-415) sends
  `providerType: selectedProviderType` — but `selectedProviderType` is module
  state (`useState<string>("")` at line 187) that is **only ever set when the
  user clicks a template card on the Templates tab** (line 1703).
- When a user opens the dialog and goes straight to the Custom tab (the only
  path for DeepSeek / GPT4All / llama.cpp / any true OpenAI-compatible service
  without a dedicated template), `selectedProviderType` is `""`.
- Backend `registry.rs:412-414` then returns
  `"Unknown provider type: "` for the empty string.
- Result: every Custom-provider submission silently fails with a confusing
  toast.

Secondary defensive gap (not the proximate cause but still real):
`crates/lr-providers/src/factory.rs:741` `validate_config` and
`crates/lr-providers/src/openai_compatible.rs:71` `OpenAICompatibleProvider::new`
do not trim whitespace from `base_url`. A URL pasted with leading/trailing
whitespace passes through validation but fails HTTP requests. Mistral's
factory already trims (`factory.rs:911-920`); OpenAI-compatible should too.

## Fix

### 1. Frontend — pass provider type explicitly

Refactor `handleCreateProvider` to take `providerType` as a parameter, so the
function is self-contained and does not depend on global state. Update
`ProviderForm`'s `onSubmit` callback signature. Update both call sites in
`providers-panel.tsx` (the Templates configure page and the Custom tab) to pass
the correct type explicitly.

### 2. Backend — trim whitespace in base_url

- `OpenAICompatibleProvider::new`: trim whitespace from `base_url` before
  stripping trailing slashes.
- `OpenAICompatibleProviderFactory::validate_config`: trim whitespace before
  scheme check, so pasting a URL with surrounding whitespace still validates.

## Tests

### Backend (TDD — Red → Green)

- `openai_compatible::tests::test_base_url_trims_whitespace`
  - Whitespace-padded URL is normalized to the trimmed form.
- `factory::tests::test_openai_compatible_validate_accepts_whitespace_base_url`
  - `validate_config` accepts `" https://api.deepseek.com/v1 "`.
- `factory::tests::test_openai_compatible_create_trims_base_url`
  - `create` returns a provider whose `base_url` has no surrounding whitespace.

### Frontend

Static verification by reading the diff — the existing e2e harness has only one
spec. Adding a Playwright spec for the Custom tab is out of scope for a fix
PR; the type flow is now explicit enough that misconfiguration shows up at
compile time.

## Final Steps (per CLAUDE.md)

1. Plan Review — re-read the implementation against this plan.
2. Test Coverage Review — confirm all new behaviour has a test.
3. Bug Hunt — re-read the diff with fresh eyes for off-by-one, race, or state
   bugs.
4. Commit — only the files I modified.
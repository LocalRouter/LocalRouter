# Plan: Bug Discovery and Fix via Test Coverage

## Context

Code review across the Rust backend identified several confirmed bugs, ranging from a **crash-inducing panic** to missing validation and logic errors. This plan adds targeted tests that expose each bug, then fixes the underlying code.

---

## Confirmed Bugs (with fixes)

### BUG 1: Panic on multi-byte UTF-8 truncation (CRITICAL)
- **Files**: `crates/lr-server/src/routes/completions.rs:403`, `crates/lr-server/src/routes/chat.rs:951`
- **Issue**: `build_flagged_text_preview()` slices a string by byte index: `&t.text[..available.saturating_sub(3)]`. If the slice boundary falls inside a multi-byte UTF-8 character (e.g. Chinese, emoji), this **panics at runtime**.
- **Fix**: Use `char_indices()` to find a safe boundary.

### BUG 3: Config validation allows NaN/Infinity in rate limit values
- **File**: `crates/lr-config/src/validation.rs:204-210`
- **Issue**: Rate limit validation checks `limit.value <= 0.0` but `f64::NAN <= 0.0` evaluates to `false`, so NaN passes validation. Similarly `f64::INFINITY` passes.
- **Fix**: Add `!limit.value.is_finite()` check.

### BUG 5: Config validation accepts whitespace-only provider/strategy names
- **File**: `crates/lr-config/src/validation.rs:83-86, 184-188`
- **Issue**: Checks `provider.name.is_empty()` but `"   ".is_empty()` is `false`, so a name consisting of only whitespace passes validation.
- **Fix**: Add `.trim().is_empty()` check.

### BUG 6: Guardrails `confidence_threshold` and `context_size` lack range validation
- **File**: `crates/lr-config/src/validation.rs` (missing)
- **Issue**: `default_confidence_threshold` is documented as 0.0-1.0 range, `context_size` as 256-4096 range, but neither has validation.
- **Fix**: Add validation in `validate_config()` for guardrails bounds.

### BUG 8: Duplicate provider name check runs after empty name check (ordering bug)
- **File**: `crates/lr-config/src/validation.rs:73-87`
- **Issue**: The loop inserts provider name into `names` HashSet at line 75, THEN checks if name is empty at line 83. This means if there are two providers with empty names, the first one gets inserted and passes (empty string is valid), but the second triggers "Duplicate provider name: " with an empty name in the error message. The empty-name check should come first.
- **Fix**: Move the `is_empty()` check before the duplicate check.

---

## Implementation Steps

### Step 1: Fix `build_flagged_text_preview` + add tests (BUG 1)
- Fixed in both `completions.rs` and `chat.rs`
- Added tests: ascii truncation, empty input, multibyte UTF-8, emoji, short text, user preference

### Step 2: Add tests for completions `validate_request` + helpers
- Tests for: valid request, empty model, temperature out of range, NaN temperature, n=0, n+streaming, NaN penalties
- Tests for: `estimate_prompt_tokens` single/multiple, `convert_prompt_to_messages` single/multiple

### Step 3: Add tests for embeddings `validate_request`
- Tests for: valid request, empty model, empty single input, empty array, array with empty string, invalid encoding format, valid formats, zero dimensions, valid dimensions

### Step 4: Fix config validation + add tests (BUGs 3, 5, 6, 8)
- Reordered empty check before duplicate check (BUG 8)
- Added `!limit.value.is_finite()` to rate limit check (BUG 3)
- Added `.trim().is_empty()` for name validation (BUG 5)
- Added `validate_guardrails_config()` for bounds validation (BUG 6)
- Added comprehensive tests

### Step 5: Add tests for `AvailableModelsSelection`
- Tests for: all, none, by_provider, by_model, case_insensitive

## Verification

```bash
cargo test --workspace
cargo clippy --workspace
```

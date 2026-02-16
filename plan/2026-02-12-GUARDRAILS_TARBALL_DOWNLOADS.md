# Tarball-based source downloads + cache-based tests

## Context

The guardrail source manager downloads regex patterns from GitHub repos (Presidio, LLM Guard). The current approach uses the GitHub API for directory listings (60 req/hr unauthenticated limit), then downloads each file individually. This hits rate limits quickly — Presidio + LLM Guard need ~15 API calls just for directory enumeration across ~200 files.

Additionally, integration tests create temp directories and re-download everything from scratch, wasting API calls and failing when rate-limited.

## Approach

### 1. Replace directory enumeration with tarball download

**File:** `crates/lr-guardrails/src/source_manager.rs`

Replace `download_path()` + `download_directory_files()` + `download_directory_files_recursive()` with a single `download_repo_tarball()` method that:

- Downloads `https://github.com/{owner}/{repo}/archive/refs/heads/{branch}.tar.gz` (CDN, no API rate limit)
- Streams through the tar.gz, extracting only files that:
  - Match one of the `data_paths` prefixes (after stripping the `{repo}-{branch}/` root dir GitHub adds)
  - Have a supported extension (`.json`, `.txt`, `.md`, `.yar`, `.yara`, `.py`, `.yaml`, `.yml`)
  - Are not `__init__.py`, `test_*`, or `conftest*`
  - Are under 1MB
- Saves matching files to `raw_dir` and returns `Vec<DownloadedFile>`
- Captures `last-modified` header from the tarball HTTP response

**Caller change in `download_and_compile_source()`:**
- Instead of looping over `data_paths` and calling `download_path()` for each, call `download_repo_tarball()` once with all `data_paths`, then parse all returned files.

### 2. Add `flate2` + `tar` dependencies

**File:** `crates/lr-guardrails/Cargo.toml`

Add:
```toml
flate2 = "1"
tar = "0.4"
```

No workspace-level changes needed — these are only used in lr-guardrails.

### 3. Fix tests to use real cache directory

**File:** `crates/lr-guardrails/tests/source_download_tests.rs`

- Use `~/.localrouter-dev/guardrails/` (via `dirs::home_dir()`) as the cache dir instead of `tempfile::tempdir()`
- If cache already has `compiled_rules.json`, read and verify those rules (no network needed)
- Only download if cache is empty or explicitly requested
- Remove the rate-limit check/skip logic — it won't be needed with tarball downloads

### 4. Keep single-file download as fallback

For `data_paths` that point to a single file (not a directory), keep the existing `raw.githubusercontent.com` direct download. The tarball is only needed when `data_paths` points to a directory.

Actually simpler: always use the tarball. One request covers all paths regardless of whether they're files or directories.

## Files to modify

| File | Change |
|------|--------|
| `crates/lr-guardrails/Cargo.toml` | Add `flate2` + `tar` deps |
| `crates/lr-guardrails/src/source_manager.rs` | Replace directory download with tarball; simplify `download_and_compile_source` |
| `crates/lr-guardrails/tests/source_download_tests.rs` | Use real cache dir, verify cached rules |

## Verification

1. `cargo test -p lr-guardrails` — unit tests pass
2. `cargo test -p lr-guardrails --test source_download_tests -- --ignored --nocapture` — downloads via tarball, extracts rules, caches them
3. Run the same test again — should use cache (no network), still pass
4. Check `~/.localrouter-dev/guardrails/presidio/compiled_rules.json` exists with valid rules

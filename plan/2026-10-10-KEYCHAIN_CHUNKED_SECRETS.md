# Keychain: chunk secrets that exceed the credential store's entry limit (#18)

## Progress

- [x] Chunking layer (`crates/lr-api-keys/src/chunked.rs`) with platform limits
- [x] `SystemKeychain` stores/reads/deletes through the chunking layer
- [x] Route direct `keyring` writers through the chunk-aware keychain
- [x] Unit tests with an in-memory backend
- [x] Plan review
- [x] Test coverage review
- [x] Bug hunt
- [x] Commit

## Problem

On Windows, ChatGPT Plus OAuth login fails:

```
Token exchange failed: OAuth browser flow error: Failed to store access token:
Internal server error: Failed to store key: Attribute 'password encoded as UTF-16'
is longer than platform limit of 2560 chars
```

Windows Credential Manager caps a credential blob at 2560 bytes
(`CRED_MAX_CREDENTIAL_BLOB_SIZE`). keyring 3.6 (`windows-native`) stores a
password as UTF-16, so one entry holds at most 1280 UTF-16 code units. The
ChatGPT access token (a JWT) is longer. Path:
`lr-oauth/src/browser/token_exchange.rs::store_tokens` → `CachedKeychain` →
`SystemKeychain::store` (`lr-api-keys/src/keychain_trait.rs`).

Other backends, from the keyring 3.6.3 source:

- macOS (`apple-native`, generic password): no size check, no practical limit.
- Linux (`linux-native`, keyutils `user` key via `add_key`): the kernel rejects
  payloads over 32767 bytes.

## Design

New module `crates/lr-api-keys/src/chunked.rs`, platform-independent:

- `RawEntryStore` — single-entry `set` / `get` / `remove -> bool` over one
  credential-store entry. `SystemKeychain` implements it with `keyring::Entry`;
  tests implement it in memory.
- `EntryLimit { max_cost, char_cost: fn(char) -> usize }` — per-entry limit and
  how a character is measured. Platform constants via `cfg`:
  - Windows: 2048 bytes (margin under 2560), cost = UTF-16 bytes per char.
  - Linux: 32000 bytes (margin under 32767), cost = UTF-8 bytes per char.
  - elsewhere: `None` (no limit).
- Format: a secret that fits is stored in the primary entry exactly as before.
  A secret that does not fit is split on `char` boundaries into parts stored at
  `"{account}#localrouter-chunk-{i}"` (1-based), and the primary entry holds a
  header `"\u{1e}localrouter-chunked:v1:{count}:{byte_len}:{fnv1a64 hex}"`.
- Marker collision: a secret that itself starts with the header prefix is
  always chunk-encoded (even if short), so a primary entry starting with the
  prefix is always a header for anything written by this code.
- Store (chunked): write parts 1..=n, then the header (commit point), then
  remove stale parts n+1.. until one is absent. Store (fits): write the primary
  entry, then remove stale parts 1.. until absent. Stale-part cleanup failure
  is logged, not returned: the new secret is already committed.
- Read: primary absent → `None`; no prefix → returned unchanged (legacy and
  short secrets); header → read every part, a missing part is an error naming
  the part, then length and fingerprint must match the header or it is an
  error (detects parts mixed from an interrupted overwrite), never a
  truncated/garbled secret.
- Delete: remove the primary entry first, then parts 1.. until absent.
- At most 128 parts; a larger secret is rejected with a clear error, and a
  header claiming more is malformed.
- `SystemKeychain` serializes its operations through a process-wide `RwLock`
  (stores/deletes exclusive, reads shared) because several `CachedKeychain`
  instances share the system store and a multi-entry write must not interleave
  with another.
- `FileKeychain` and `MockKeychain` are unchanged (no limits; file format
  untouched).

### Writers routed through the chunk-aware path

- Everything using `CachedKeychain::system()/auto()` → `SystemKeychain` (OAuth
  tokens in lr-oauth / lr-providers / lr-mcp, provider keys in
  `lr-providers/src/key_storage.rs`, MCP auth secrets in
  `src-tauri/src/ui/commands_mcp.rs`, client secrets, …) — covered by the
  `SystemKeychain` change.
- `crates/lr-api-keys/src/keychain.rs` (direct `keyring` calls) → delegates to
  `SystemKeychain`.
- `src-tauri/src/main.rs` and `src-tauri/src/ui/commands_marketplace.rs`
  (marketplace MCP bearer tokens written with `keyring::Entry`) →
  `CachedKeychain::auto()`, the same keychain their readers use.
- Left: `src-tauri/examples/check_keychain_entries.rs` (diagnostic example that
  writes a short test password).

## Tests (in-memory backend enforcing the limit)

Short secret, exactly at the limit, one over the limit, multibyte characters
at part boundaries (UTF-16 cost incl. surrogate pairs), ChatGPT-sized token
under the real Windows rule, overwrite long→short and short→long and
long→shorter-long (stale parts removed), delete of a chunked secret, missing
part, mismatched parts, malformed header, legacy single entry, marker-collision
secret, oversized secret rejected, unlimited platform behaviour.

## Final steps

1. Plan review — compare implementation against this plan. Done: every item
   above is implemented as described.
2. Test coverage review — every branch in `chunked.rs` exercised. Added tests
   for the two branches the first pass missed: a limit smaller than one
   character, and a failed leftover-part cleanup (store still succeeds, delete
   reports the failure). A platform test round-trips a 100 000-char token
   through `PLATFORM_LIMIT` (and on Windows asserts every entry satisfies
   keyring's UTF-16 blob rule).
3. Bug hunt — re-read with fresh eyes (boundaries, ordering, error paths).
   Found and fixed: `load` reserved `String::with_capacity(byte_len)` straight
   from the header, so a corrupt header could request an unbounded allocation;
   the reservation is now capped at 1 MiB (regression test added). Checked:
   parts are never empty, the header is checked against the limit before any
   write, an interrupted chunked write over a plain secret leaves the old
   secret readable, the global lock is never taken re-entrantly.
4. Commit (not pushed for this task, per instructions).

## Verification

- `cargo test -p lr-api-keys`, `cargo clippy -p lr-api-keys --all-targets -- -D warnings`,
  `cargo clippy -p localrouter --all-targets -- -D warnings`,
  `cargo fmt --all -- --check` (rustup stable 1.99).
- The `x86_64-pc-windows-gnu` std target is installed, but `cargo check` for it
  stops at `ring`'s build script (no mingw C compiler). `chunked.rs` itself was
  type-checked with `rustc --emit=metadata --target {x86_64-pc-windows-gnu,
  x86_64-unknown-linux-gnu, aarch64-apple-darwin}` (lib and `--test`, with
  `deny(warnings)`) against stubbed `lr_types`/`zeroize`/`tracing`.
- Not verified: a real login against Windows Credential Manager.

## Not in scope

- The marketplace installers (`src-tauri/src/main.rs`,
  `src-tauri/src/ui/commands_marketplace.rs`) store a bearer token at account
  `{server_id}`, but `lr-mcp/src/manager.rs` reads bearer tokens from
  `{server_id}_bearer_token` (and the config's `token_ref` stays `"pending"`),
  so a marketplace-installed bearer token is never found. Pre-existing and
  unrelated to entry size; account names left unchanged here.

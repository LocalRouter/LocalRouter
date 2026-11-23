# Native server, skills and platform leftovers

Leftovers from `plan/2026-11-16-STREAMING_REQUEST_PIPELINE.md` ("Phase 5 as built",
"Not done") and `plan/2026-11-22-SKILLS_CONFIG_DOWNLOAD_FOLLOWUPS.md` ("Left as is"),
plus cross-platform compile checks.
Branch: `fix/server-skills-platform-leftovers`.

## Checklist

- [x] Save this plan
- [x] 1 Secret boundary on non-inference POST routes (Redact/Ask/Notify)
- [x] 2 Audio/image multipart: bounded fields, prompts scanned
- [x] 3 Configurable large-body budget, threshold and wait (config, validation, UI, types, mock)
- [x] 4 Content-Length on forwarded parsed bodies
- [x] 5 Typed-field errors do not echo request values
- [x] 6 Marketplace update gap: bounded reader retry where no atomic exchange exists
- [x] 7 `lr-engines` pid records written atomically
- [x] 8 Cross-platform clippy/check (Windows MSVC, Linux GNU), fixes, proposed CI job
- [x] 9 Bug hunt in touched files
- [x] Plan Review
- [x] Test Coverage Review
- [x] Bug Hunt
- [x] Validation, commit per item, push

## Findings (verified at 0d44277b)

1. Confirmed. `secret_placeholders::run_scoped` treats only `chat/completions`,
   `completions`, `responses`, `messages`, `systemone`, `decisions` as inference; every
   other POST in Redact/Ask is forwarded with the warning "Secret Scan does not support
   this endpoint". Notify is not applied to them at all (only the chat pipeline and
   System One call the scanner). POST routes on the main router carrying user text that
   reaches a provider: `embeddings` (`input`), `moderations` (`input`), `rerank`
   (`query`, `documents`), `search` (`query`), `ocr` (`filename`; the document is a URL
   or base64 binary and URLs are not scanned on chat either), `images/generations`
   (`prompt`), `audio/speech` (`input`), `videos` (`prompt`, JSON or multipart),
   `messages/count_tokens` (Messages shape). Not content routes: `realtime/client_secrets`
   (only `type`/`model`), `files` (stored locally; batch rows are scanned by
   `batch_jobs::policy` before submission), `batches*` (ids/control), `videos/{id}/*`.
   Audio transcription/translation and image edits sit on separate routers without the
   secret boundary at all; their `prompt` fields were never scanned.
2. Confirmed. The handlers read each multipart field whole (`field.bytes()`,
   `field.text()`) with no per-field bound beyond the route limit (25/50 MiB), and the
   audio file is copied once more (`.to_vec()`).
3. Confirmed: `LARGE_BODY_THRESHOLD` (8 MiB), `LARGE_BODY_BUDGET` (512 MiB), `BUDGET_WAIT`
   (3 s) are constants in `body_limits.rs`.
4. Verified: nothing downstream reads the request `Content-Length` of a parsed body.
   `GatewayJson` takes the value; axum `Json`/`Bytes` read the body to its end (the
   lazy body reports its exact size once serialized); handlers build new provider
   requests (reqwest computes its own length). The lazy body's `size_hint` is unknown,
   which is correct before serialization. Decision: keep it dropped (no consumer needs
   it); a test covers a byte reader with a body limit reading a parsed request whole.
5. Confirmed: `GatewayJson` maps `from_value` errors to `Invalid request: {serde error}`,
   which includes values (`invalid type: string "…"`, `unknown variant `…``). The axum
   `Json` extractor used by embeddings, moderations, image generations and speech echoes
   the same way (plain-text rejection).
6. Confirmed: readers (`lr-skills` discovery, `get_resource`, script path validation) do
   not retry; on Windows (no atomic exchange) a read between the two renames sees no skill.
7. Confirmed: `supervisor.rs::write_pid_records` uses `std::fs::write`.

## Decisions

1. New `crates/lr-server/src/content_scan.rs`: the list of content routes and a
   chat-shaped scan projection per route (`{"messages":[{"role":"user","content":…}]}`;
   `messages/count_tokens` is scanned as itself). In `run_scoped` a JSON content route is
   read and parsed like an inference route, then:
   - Redact/Ask: scanned through `scan_request_for_secrets_with_source` with the vault in
     scope (Ask popups, remembered decisions, monitor events as for chat), masked with
     `secret_context::protect`, then *verified* (`content_scan::verify_masked`): the
     scanner runs again over the projection of the masked request; a finding that is not a
     remembered "ignore" and that the vault knows (or cannot say it does not), or any such
     finding in Redact (Redact registers every new finding), means a secret survived
     masking (a field masking skips, masking that failed, a mapping that could not be
     stored), and the request is refused (403 `secret_scan_unmaskable`) instead of being
     forwarded. (Re-running `protect` as the check was rejected: `Vault::protect` reports
     its own failures as success and leaves the value unchanged.)
   - Notify: scanned (popup, monitor event), remembered redactions applied, forwarded.
   - Responses are restored only when JSON or SSE (speech audio passes untouched).
   Multipart content routes (`audio/transcriptions`, `audio/translations`,
   `images/edits`, multipart `videos`) get the vault scope from the middleware without
   reading the body; the handler scans its text fields after parsing with
   `content_scan::scan_form_fields` (same policy; Redact/Ask without a vault scope is
   refused rather than forwarded). The secret boundary layer is added to the audio and
   image routers (one line each in `lib.rs`).
2. Text form fields are read chunk by chunk with a 128 KiB cap (image edit prompts may
   be 32,000 characters) and refused with 400 when exceeded; image parts stop at their
   20 MB limit instead of being buffered whole first; file parts are read once without
   the extra `to_vec` copy. Route limit and budget are unchanged.
3. `server.large_body_threshold_bytes` (default 8 MiB, 1–256 MiB),
   `server.large_body_budget_bytes` (default 512 MiB, 16 MiB–8 GiB, at least the
   threshold), `server.large_body_wait_ms` (default 3000, 0–60000). Read live per request;
   a changed budget resizes the one semaphore in place: growing adds permits, shrinking
   removes the free ones and records the rest as a debt that in-flight reservations pay
   on release, so nothing new is admitted until the bytes held fit the new budget (see
   "As built (small fixes)").
   Exposed through `get_server_config`/`update_server_config`, settings UI in a "Large
   Request Memory" card directly below the "Request Limits" card.
4. See finding 4.
5. `serde_path_to_error` (already in the lock file via axum) gives the field path;
   messages are rewritten to the path plus the expected type, never the value. Used by
   `GatewayJson` and a new `TypedJson` extractor that replaces axum `Json` on the
   embeddings, moderations, image generation and speech handlers (no ingress
   normalization added to them). Their type errors become 400 (axum answered 422).
6. `lr_utils::fs::retry_while_missing`: retries `NotFound` a bounded number of times
   (10 × 10 ms) while a `lr_utils::fs::PathReplace` guard is held in this process or
   one finished during the attempt; otherwise a single attempt. `promote` holds the
   guard from before the exchange (or the two-rename fallback, used on Windows and on
   filesystems without exchange support) until the staging directory with the old
   version is removed. The decision is made at runtime, not per platform (see "As built
   (small fixes)"). Used by the `lr-skills` readers: skill loading (waits for a listed directory that vanished;
   a directory without `SKILL.md` is not waited for), `get_resource` and script path
   resolution. A scan whose directory *listing* falls in the gap omits the skill until
   the watcher's rescan after the second rename.
7. `lr_utils::fs::write_atomically` (`<name>.<pid>.part` + rename) used by
   `write_pid_records`.
8. See "Cross-platform checks".

## Cross-platform checks

`rustup target add x86_64-pc-windows-msvc x86_64-unknown-linux-gnu` (stable), then from
macOS (aarch64) a full `cargo clippy --workspace --all-targets --target <t> -- -D warnings`
for both targets — every crate, including `localrouter` (src-tauri) and its tests. No
crate had to be excluded. Clippy only checks (no linking), but build scripts still
compile C and probe system libraries, which needed stand-ins (all local, no downloads):

- C/C++ for both targets: `zig cc`/`zig c++` (Homebrew zig, bundled libc headers;
  mingw headers stand in for the Windows SDK) through wrappers that drop the
  `--target=` cc-rs adds; archiver `zig ar` (Linux) / `zig lib` (MSVC). `ring` and
  `aws-lc-sys` (with `AWS_LC_SYS_PREBUILT_NASM=1`) build their objects this way.
- Linux pkg-config (gtk/webkit/soup/… for Tauri): `PKG_CONFIG_LIBDIR` pointing at
  placeholder `.pc` files (version 99, `-lc`), so the host's macOS `.pc` files are never
  used (they emit `-framework` flags); OpenSSL headers from Homebrew via
  `X86_64_UNKNOWN_LINUX_GNU_OPENSSL_DIR`.
- Windows resources (`tauri-winres`): Homebrew `llvm-rc` on `PATH`.

Scripts used (kept outside the repo): `cross-linux.sh`, `cross-win.sh <triple>`; the
generated `src-tauri/gen/schemas/{linux,windows}-schema.json` from the cross builds
were discarded.

Results:
- **x86_64-unknown-linux-gnu**: clean (after merging master, which already fixed a
  `websocket_stream_tests.rs` initializer the first run tripped over).
- **x86_64-pc-windows-msvc**: three findings, fixed —
  - `lr-utils/src/binary.rs` `is_executable_file`: `clippy::needless_bool` on
    non-Unix (`if !is_file { return false } … true`); rewritten per platform.
  - `src-tauri/tests/push_relay_gateway_tests.rs`: the stdio upstream fixture and two
    imports are used only by `#[cfg(unix)]` tests → `dead_code`/`unused_imports` on
    Windows; `#[cfg_attr(not(unix), allow(...))]` on each (MCP gateway area: one line
    each).
  - `lr-monitoring`: `dirs` is unused on Windows (cargo `unused_dependencies` manifest
    warning; the Windows log directory comes from `LOCALAPPDATA`) → moved to a
    `cfg(not(target_os = "windows"))` dependency.
- Not platform-specific (reported on every target, cargo manifest lint, not failing
  `-D warnings`, left alone): `lr-guardrails` optional `tauri` and `lr-monitor`
  `tracing` "unused dependency".
- macOS (host): the normal host clippy run.

### Proposed CI job (not added; CI minutes cost money)

The existing `lint` job runs on Linux only, so code that is dead or unused outside
Linux (macOS-only tray code was caught only after release in the other direction) and
Windows-only lint failures reach a release unseen. Native runners are the simplest
option; on GitHub-hosted runners Windows minutes bill at 2× and macOS at 10× the Linux
rate for private repositories (free for public ones). Clippy only checks, so a warm
`rust-cache` keeps each run to a few minutes. The macOS leg can be dropped if the
release build (which already runs on macOS) is considered enough.

```yaml
  # Compile-only lint on the platforms the Linux lint job cannot see.
  lint-cross:
    name: Lint (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    timeout-minutes: 45
    strategy:
      fail-fast: false
      matrix:
        os: [macos-latest, windows-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version-file: '.nvmrc'
          cache: 'npm'
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
        with:
          shared-key: ci-lint-${{ matrix.os }}
      - name: Cache model catalog
        uses: actions/cache@v4
        with:
          path: crates/lr-catalog/catalog/
          key: catalog-${{ github.run_id }}-lint-${{ matrix.os }}
          restore-keys: catalog-
      - name: Install frontend dependencies
        run: npm ci
      - name: Run Clippy
        run: cargo clippy --workspace --all-targets -- -D warnings
```

A Linux-only alternative (no macOS/Windows runner minutes) is to cross-check from the
existing lint job the way it was done locally: install `zig` and `llvm` (for
`llvm-rc`), add the `x86_64-pc-windows-msvc` target, and run
`cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings`
with the `CC_*`/`CXX_*`/`AR_*` wrappers and `AWS_LC_SYS_PREBUILT_NASM=1`. It costs only
Linux minutes but depends on those stand-ins staying compatible with the dependencies'
build scripts.

## As built

Commits (branch `fix/server-skills-platform-leftovers`):
`0deb50f8` (7), `c005d261` (6), `5917a9ee` (3), `7c56cae8` (5 and 4), `0b496878` (1 and
2), `d1517160` (8), plus a final commit with the handler wiring test and this record.

1. `crates/lr-server/src/content_scan.rs` (routes, projections, `verify_masked`,
   `scan_form_fields`, `with_form_vault`, bounded form readers);
   `secret_placeholders.rs::run_scoped` (content routes read, scanned, masked, verified;
   Notify scan; `run_form`; `restore_text_response`); `lib.rs` (module + the boundary
   layer on the audio and image routers). Tests: `content_scan_tests.rs` (classification,
   projections, form readers) and `secret_placeholders_content_tests.rs` (every content
   route in Redact/Notify/Off, Ask deny/replace/allow, binary passthrough, compressed
   bodies, unmaskable refusal, multipart masking with restoration, the real audio and
   image edit handlers).
2. `routes/audio.rs`, `routes/images.rs`, `video_jobs/mod.rs`: bounded field reads,
   prompt scanning before dispatch (images: before the monitor event, which records the
   prompt). Multipart video scans in `create` when the body was a form.
3. `lr-config` (`ServerConfig` fields, `LargeBodyLimits`, validation,
   `ConfigManager::large_body_limits`), `body_limits.rs` (live settings, budget
   replacement), `src-tauri/src/ui/commands.rs`, `src/types/tauri-commands.ts`,
   `src/views/settings/server-tab.tsx`, `HowToConnect.tsx` (uses `ServerConfigInfo`),
   demo mock and mock data.
4. No code change; test in `gateway_json.rs`.
5. `routes/gateway_json.rs` (`typed`, `describe`, `TypedJson`), handlers for embeddings,
   moderations, image generation, speech.
6. `lr-utils/src/fs.rs` (`retry_while_missing`, `ATOMIC_EXCHANGE_PLATFORM`),
   `lr-skills` `discovery.rs`, `manager.rs`, `executor.rs`.
7. `lr-utils/src/fs.rs` (`write_atomically`), `lr-engines/src/supervisor.rs`.
8. See "Cross-platform checks".

Plan Review: every item implemented. Deviations: the masking check re-scans instead of
re-protecting (see Decision 1); text fields are capped at 128 KiB, not 64 KiB; the
large-body settings got their own card below "Request Limits".

Test Coverage Review: new branches are covered except — the Windows-only retry in a
real skill read (`resource_read_waits_out_a_two_rename_update` returns early on
macOS/Linux; the retry loop itself is covered platform-independently in `lr-utils`);
multipart video scanning (its handler refuses before the scan without a configured
provider; `scan_form_fields` is covered directly); the settings card (no frontend unit
test; type-checked and built).

## Bugs found

1. (Security, item 1) Redact/Ask forwarded user text on embeddings, moderations,
   rerank, search, OCR, image generation, speech, video and token counting unscanned;
   Notify never notified for them; audio and image edit prompts were never scanned.
   Fixed (`0b496878`).
2. Image edit prompts were recorded in plaintext in the monitor event before any scan
   could run; the scan now precedes the event (`0b496878`).
3. Image parts were buffered whole before their 20 MB check; multipart text fields had
   no bound below the route limit; audio files were copied once more (`0b496878`).
4. Typed request errors echoed request values (`GatewayJson` and the axum `Json`
   extractor on four handlers) (`7c56cae8`).
5. Settings: saving the request body limit reloaded the whole form and discarded unsaved
   host/port edits (`5917a9ee`).
6. `lr-utils` `is_executable_file` failed clippy (`needless_bool`) on Windows;
   `push_relay_gateway_tests` failed clippy (`dead_code`, `unused_imports`) on Windows;
   `lr-monitoring` had an unused dependency on Windows (`d1517160`).
7. Pid records could be torn by a crash mid-write (`0deb50f8`); skill readers failed
   inside the Windows update gap (`c005d261`).

Checked without finding bugs: `body_limits` permit accounting across chunk waits,
`gateway_json` lazy body and media-type rule, marketplace `promote`/`recover`,
`write_atomically` failure cleanup.

## Validation

All with the rustup stable toolchain (1.100.0), shared target dir.

- `cargo fmt --all -- --check` — clean
- `cargo clippy --workspace --all-targets -- -D warnings` (aarch64-apple-darwin) — clean
- `cargo clippy --workspace --all-targets --target x86_64-unknown-linux-gnu -- -D warnings` — clean
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings` — clean after the fixes above
- `cargo test -p lr-utils -p lr-engines -p lr-skills -p lr-marketplace -p lr-config -p lr-server -p lr-monitoring` — all pass
- `cargo test -p localrouter --lib --bins` — 497 passed, 12 ignored (each)
- `cargo test -p localrouter --test push_relay_gateway_tests --test audio_endpoint_tests --test image_edits_endpoint_tests --test unified_api_tests --test openapi_tests` — all pass
- `npx tsc --noEmit`, `npm run test:unit`, `npm run build`, `npm --prefix website run build:check`, `node scripts/check-demo-mock-coverage.mjs` — pass

## Remaining / reviewer notes

- Masking check in Redact treats every non-ignored finding still present after masking
  as a leak. If the scanner ever flags text inside a placeholder or a remembered
  redaction marker, such requests would be refused; the route tests show it does not
  today.
- `retry_while_missing` sleeps on the calling thread (≤100 ms, only while a skill is
  being replaced); skill reads run on async tasks.
- Type errors on embeddings, moderations, image generation and speech are now 400
  (axum answered 422 for data errors).
- The image edit handler checks access before it scans (see "As built (small fixes)").

## As built (small fixes)

Branch `fix/server-skills-small`: `2773d331` (1), `5d1c306e` (2), `a64632f7` (3),
`0d8143f1` (bug 1 below), plus this record. Each of the three reviewer notes was
verified in the code before fixing.

1. Atomic-exchange fallback detected at runtime. `lr_utils::fs::PathReplace` (a guard
   over a process-wide counter of replaces in progress and finished) replaces the
   compile-time `ATOMIC_EXCHANGE_PLATFORM` flag, which is removed. `StagedInstall::promote`
   takes the guard before trying the exchange and keeps it in the `StagedInstall` until
   its drop has removed the staging directory (struct fields drop after `Drop::drop`).
   `retry_while_missing` retries a `NotFound` only when a replace was active or finished
   during the attempt (a `finished` snapshot is taken before each attempt, so a replace
   that ends between the failed read and the check still counts). A path that is just
   missing costs two atomic loads and no sleep, on every platform (Windows used to wait
   100 ms for every missing path).
2. `routes/images.rs::image_edits`: parse and validate the form, then
   `check_image_access`, then `scan_form_fields`, then the monitor event with the
   scanned prompt. A refusal is recorded with the prompt left out (it was not scanned).
   This matches chat's pipeline order (access → rate limits → secret scan). The prompt
   length limit now applies to the prompt as sent, before masking.
3. `body_limits.rs`: one semaphore for the server's lifetime. `Budget::resize` grows by
   paying the debt first and then adding permits; it shrinks by forgetting free permits
   and recording the remainder as `debt`. Requests hold a `Reservation` whose drop pays
   the debt (splitting and forgetting that many permits) under the budget lock before it
   returns the rest. `Budget::settle` runs before each reservation and takes back permits
   that an abandoned waiter returned straight to the semaphore while debt was owed. A
   body being read caps its reservation at the current budget, so a body larger than a
   budget that shrank meanwhile still runs alone.

### Plan Review

All three items and the plan-text corrections (Decisions 3 and 6, reviewer notes) are
done. Not done: the Secret Scan boundary (`secret_placeholders::run_scoped`) still scans
and masks JSON routes (chat in Redact/Ask, and the JSON content routes such as
embeddings and image generation in every action) before their handlers check access,
so an Ask popup can still open for a JSON request that is then refused. Fixing that
means an access pre-check in the boundary for every route, which is larger than this
change and touches the inference path other work is on.

### Test Coverage Review

- `lr-utils` `fs.rs`: retries only during a replace (bounded, other errors not
  retried), a missing path without a replace fails at once (including after a replace
  that finished before the read), a replace that finishes between the failed read and
  the check is retried, and a real two-rename replace under `PathReplace` is waited for.
- `lr-marketplace` `staging.rs`: `readers_wait_out_a_two_rename_update` forces the
  fallback (a thread-local test switch skips the exchange) and runs on every platform;
  without the guard it fails (checked: 6156 failed reads). The exchange test now reads
  through `retry_while_missing`.
- `lr-skills` `resource_read_waits_out_a_two_rename_update` now runs on every platform.
- `lr-server` `content_tests::a_refused_image_edit_opens_no_secret_scan_popup`: Ask
  (inside a form vault scope, so the scan could mask and would ask) and Notify, a 403
  that is not `secret_scan_unmaskable`, no pending popup, no monitor event containing
  the secret. With the old order the Ask case times out (checked).
- `lr-server` `body_limits_tests.rs`: shrinking below held bytes admits nothing (declared
  and chunked) until held bytes fit; growing pays the debt first; a body outgrowing a
  shrunk budget runs alone; permits of an abandoned waiter pay the debt; a 4-thread
  stress test with 8 workers and 400 resizes between 16 and 64 permits never holds more
  than 64 and ends with no debt and every permit back.

### Bug Hunt

1. (Fixed, `0d8143f1`) Even with the atomic exchange, a reader could get `NotFound`
   during an update: a lookup that resolved the skill directory to the old version just
   before the exchange then looked up a file in it after the staging cleanup removed it.
   `an_update_never_leaves_the_skill_directory_missing` failed about one run in six
   (4 of 25 runs, with or without the new test); 0 of 25 after the fix. The guard now
   spans the cleanup, and the test reads as skill readers do (with the retry).
2. (Fixed, `a64632f7`) A waiter that gives up returns the permits it was partly
   assigned straight to the semaphore, bypassing any debt; `settle` reclaims them.
3. (Fixed, `a64632f7`) A body that crossed the threshold before a shrink kept the old
   budget as its cap and could wait for permits the budget no longer had, ending in a
   503 after the wait; it now caps at the current budget.
4. (Fixed, `5d1c306e`) Besides the popup, a refused image edit recorded the prompt in
   its monitor event; it now leaves the unscanned prompt out.
5. (Not fixed, see Plan Review) Boundary scans before access on JSON routes.

Checked without finding bugs: `Reservation` drop and `resize` lock ordering (the
holder mutex is never held while the budget lock is taken, and no permit is dropped
inside `resize`), `promote` rollback with the guard held, guard drop order on the
`keep` path.

### Validation (small fixes)

Rustup stable toolchain, shared target dir, `LOCALROUTER_SKIP_CATALOG_FETCH=1`.

- `cargo fmt --all -- --check` — clean
- `cargo clippy --workspace --all-targets -- -D warnings` (aarch64-apple-darwin) — clean
  (two existing `unused dependency` manifest notes for `lr-guardrails` and `lr-monitor`)
- `cargo test -p lr-utils -p lr-skills -p lr-marketplace -p lr-server` — all pass
  (71, 82 + 1 ignored, 46, 353); the doc tests failed to link (`can't find crate`)
  while other worktrees rebuilt the shared target and passed when rerun with `--doc`
- `cargo test -p localrouter --test image_edits_endpoint_tests` — pass
- `lr-marketplace` lib tests 25 runs in a row — all pass
- Not run: Linux and Windows cross-target clippy. Cross builds of these crates fail in
  this environment on C build scripts (`ring`, `aws-lc-sys`, `glib-sys`,
  `libdbus-sys`) before reaching our code. The change adds no platform-specific code
  and removes the only `cfg`-dependent constant it touched.

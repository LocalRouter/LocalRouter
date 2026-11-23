# Native server, skills and platform leftovers

Leftovers from `plan/2026-11-16-STREAMING_REQUEST_PIPELINE.md` ("Phase 5 as built",
"Not done") and `plan/2026-11-22-SKILLS_CONFIG_DOWNLOAD_FOLLOWUPS.md` ("Left as is"),
plus cross-platform compile checks.
Branch: `fix/server-skills-platform-leftovers`.

## Checklist

- [x] Save this plan
- [ ] 1 Secret boundary on non-inference POST routes (Redact/Ask/Notify)
- [ ] 2 Audio/image multipart: bounded fields, prompts scanned
- [ ] 3 Configurable large-body budget, threshold and wait (config, validation, UI, types, mock)
- [ ] 4 Content-Length on forwarded parsed bodies
- [ ] 5 Typed-field errors do not echo request values
- [ ] 6 Marketplace update gap: bounded reader retry where no atomic exchange exists
- [ ] 7 `lr-engines` pid records written atomically
- [ ] 8 Cross-platform clippy/check (Windows MSVC, Linux GNU), fixes, proposed CI job
- [ ] 9 Bug hunt in touched files
- [ ] Plan Review
- [ ] Test Coverage Review
- [ ] Bug Hunt
- [ ] Validation, commit per item, push

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
   which is correct before serialization. Decision: keep it dropped; add a test that the
   serialized body still reaches a `Bytes` consumer and that its size hint is exact after
   serialization is not needed. (See "As built" for what changed.)
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
     `secret_context::protect`, then *verified*: the projection of the masked request is
     protected again; if that changes anything (a registered secret survived masking,
     e.g. in a field masking skips, or masking failed), the request is refused (403
     `secret_scan_unmaskable`) instead of being forwarded.
   - Notify: scanned (popup, monitor event), remembered redactions applied, forwarded.
   - Responses are restored only when JSON or SSE (speech audio passes untouched).
   Multipart content routes (`audio/transcriptions`, `audio/translations`,
   `images/edits`, multipart `videos`) get the vault scope from the middleware without
   reading the body; the handler scans its text fields after parsing with
   `content_scan::scan_form_fields` (same policy; Redact/Ask without a vault scope is
   refused rather than forwarded). The secret boundary layer is added to the audio and
   image routers (one line each in `lib.rs`).
2. Text form fields are read chunk by chunk with a 64 KiB cap (413-style 400 when
   exceeded); file fields are read once without the extra copy. Route limit and budget
   are unchanged.
3. `server.large_body_threshold_bytes` (default 8 MiB, 1–256 MiB),
   `server.large_body_budget_bytes` (default 512 MiB, 16 MiB–8 GiB, at least the
   threshold), `server.large_body_wait_ms` (default 3000, 0–60000). Read live per request;
   a changed budget replaces the semaphore for new requests (in-flight requests keep and
   release permits of the old one, so the transient total is bounded by old + new).
   Exposed through `get_server_config`/`update_server_config`, settings UI in the
   "Request Limits" card.
4. See finding 4.
5. `serde_path_to_error` (already in the lock file via axum) gives the field path;
   messages are rewritten to the path plus the expected type, never the value. Used by
   `GatewayJson` and a new `TypedJson` extractor that replaces axum `Json` on the four
   handlers above (no ingress normalization added to them).
6. `lr_utils::fs::retry_while_missing`: retries `NotFound` a bounded number of times
   (10 × 10 ms) on targets without `exchange_paths` support (not macOS/Linux); a single
   attempt elsewhere. Used by the `lr-skills` readers.
7. `lr_utils::fs::write_atomically` (`<name>.<pid>.part` + rename) used by
   `write_pid_records`.
8. See "Cross-platform checks".

## Cross-platform checks

(filled in as run)

### Proposed CI job (not added; CI minutes cost money)

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

## As built

(filled in per item)

## Bugs found

(filled in)

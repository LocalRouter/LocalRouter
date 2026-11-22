# Skill install hardening: extraction budgets and atomic installs

Follow-up to `plan/review-2026-10-01/mcp-tools-context.md` ("Skill ZIPs" and
"Transactional installs") and item 1 of the follow-ups in
`plan/review-2026-10-01/security-storage-models-build.md`.

## Findings before the change

Two code paths put skill files on disk:

1. **Skill archives** (`.zip` / `.skill` configured as a skill source) —
   `crates/lr-skills/src/discovery.rs::discover_from_zip` extracts into
   `<temp>/localrouter-skills/<stem>-<hash>/`.
   - Lexical traversal was blocked by `enclosed_name()`; an unsafe entry was
     skipped and the rest of the archive was still installed.
   - No budget of any kind: total bytes, per-file bytes, entry count, ratio.
   - Symlink entries were written as regular files holding the link target.
   - Per-entry I/O errors were logged and ignored, and the directory was
     created before extraction, so an interrupted or failed extraction left a
     half-extracted directory that every later scan reused as complete.
   - The archive was read fully into memory to hash it.
   - Cleanup of superseded extractions matched `<stem>-*`, so `foo.zip`
     deleted the live extraction of `foo-bar.zip`.
2. **Marketplace installs** — `lr-marketplace` downloads each file straight
   into `marketplace-skills/<source>/<name>/`.
   - Already handled: path confinement, symlink checks, HTTP status checks,
     16 MiB per file and 64 MiB per install enforced on received bytes.
   - Missing: file-count limit, duplicate / case-colliding manifest paths,
     depth and length limits; errors did not say which limit was hit.
   - Not atomic: a failed download left a partial directory, and an update
     overwrote files in place (mix of old and new, stale files kept).
   - No serialization of concurrent installs of the same skill.

Model downloads: `lr-embeddings` and `lr-compression` copy each file from the
Hugging Face cache straight to its final path, and `is_downloaded` only checks
presence, so an interrupted copy leaves a truncated file that counts as
downloaded. `lr-engines` release downloads are already staged (`.tmp-<run>`,
backup, rename) and checksum-verified.

## Limits

| Limit | Archive (`lr-skills`) | Marketplace download |
|---|---|---|
| Total bytes written | 256 MiB | 64 MiB (existing) |
| Bytes per file | 64 MiB | 16 MiB (existing) |
| Entries / files | 10,000 | 1,000 |
| Path depth | 32 | 32 |
| Path length | 1,024 bytes | 1,024 bytes |
| Path component length | 255 bytes | 255 bytes |
| Expansion ratio | 100x the archive file, checked past 16 MiB | n/a |

All byte limits count what is actually written; declared sizes only allow an
earlier rejection.

## Checklist

- [x] Save this plan
- [x] `lr-skills/src/archive.rs`: bounded, strict extraction (limits above;
      traversal, absolute, drive/stream, backslash, NUL, link, special-file,
      duplicate and case-colliding entries fail the whole archive)
- [x] Staged extraction: `.staging-<uuid>` beside the final directory, renamed
      into place only when complete; removed on failure; stale ones removed at
      startup; extraction serialized per target
- [x] Discovery: failed extraction reports an error per source, keeps serving
      the previously extracted version of the archive when there is one, and
      no longer deletes extractions of other archives
- [x] Completion marker (`.localrouter-extraction`: file count and bytes)
      written into each extraction; reuse requires the tree to still match it,
      otherwise the archive is extracted again (covers OS pruning of the temp
      directory and extractions written before this change); the name is
      reserved inside archives
- [x] Hash archives by streaming
- [x] Surface the per-source error in `list_skill_sources` (Rust struct, TS
      type, demo mock, source detail view)
- [x] Marketplace: file-count, duplicate, depth and length checks; errors name
      the limit
- [x] Marketplace: download into `marketplace-skills/.staging/<uuid>/new`,
      validate `SKILL.md`, swap by rename, keep the previous version until the
      swap succeeds, roll back on failure, recover at startup, serialize per
      skill; deleting a marketplace skill takes the same per-skill lock
- [x] Model downloaders: copy to a `.part` sibling and rename (small analogous
      fix); multi-file manifest left as a follow-up
- [x] Tests: lying headers, file count, traversal, symlink, special file,
      collisions, failed extraction leaves nothing, previous version kept,
      atomic update, stale staging cleanup, crash recovery, concurrent installs
- [x] Plan Review: compare this plan with the implementation
- [x] Test Coverage Review: every new error path has a test
- [x] Bug Hunt: re-read the new code for races, off-by-one and cleanup gaps
- [x] Validate (fmt, clippy, crate tests, frontend type check), commit, push

## Left as follow-ups

- Archive extractions still live in the OS temp directory (staging is beside
  them on the same filesystem). A pruned extraction is now detected and
  re-extracted, but moving extractions into the data directory would also
  need cleanup of extractions of archives no longer configured, which the OS
  temp directory provides today.
- `lr-engines` archive extraction has no byte budget; its inputs are pinned,
  checksum-verified releases of several GiB, so limits need separate sizing.
- Embedding / compression models have no manifest tying their files to one
  revision; only the per-file truncation window is closed here.

# Index / Skill tool efficiency

Source: analysis of Claude Code (`~/.claude/projects`) and Codex
(`~/.codex/sessions`, `~/.codex/archived_sessions`) history for
`SkillRead`, `IndexSearch`, `IndexRead` usage (2026-10-08).

## Findings

- **Codex never used any LocalRouter tool.** Its MCP client (rmcp) fails the
  handshake: LocalRouter answers `notifications/initialized` with
  `{"jsonrpc":"2.0","id":null,"result":null}` (gateway.rs notification branch,
  returned as an HTTP body by `routes/mcp.rs::send_response`). rmcp cannot
  parse an `id:null` result and closes the transport.
- **Claude Code: compression is mostly wasted.** 56 of 175 proxied responses
  were compressed (threshold 1000 B, 200 B preview); 49 were followed by
  `IndexRead`, 46 of which read the whole document back. ~45 extra round
  trips and ~31K chars more than sending the responses uncompressed.
  21 compressed payloads were under 2 KB.
- **SkillRead was compressed in 9 of 11 calls** — the instructions the call
  exists to deliver came back as a 200-byte preview.
- **IndexSearch `source` leaks**: vector search ran over every source and was
  RRF-merged in, so 78% of scoped-search output was off-source or duplicate. A
  nonexistent source returned unrelated hits instead of an error.
- **Ranking**: overlap dedup kept the *worse* BM25 hit and sorted worst-first.
- Hints name generic `read(source, …)` / `search(…)` instead of the configured
  tool names; `IndexRead` has no continuation hint; 500-char sub-lines give
  headers like `lines 1-1-1-14 of 1`; cross-query duplicates inflate output
  past the original size.
- Activated catalog tools never became callable in Claude Code.

## Plan

### Phase 0 — correctness
1. JSON-RPC notifications get `202 Accepted` with an empty body (HTTP and SSE
   paths); never emit `id:null` result responses. Route-level regression test.
2. Dedup keeps the better (more negative) rank; results sorted best-first.
   Ordering test.
3. Vector search respects `source` prefix and date range; vector-only hits are
   trimmed into line-numbered snippets. Unknown `source` → explicit message
   listing indexed labels.
4. Compressed skill catalog hint only points at `catalog:skills` when that
   source is indexed.
5. Response labels stay unique across session/server resets.
6. Compression strips `structuredContent` so the full payload cannot slip past.

### Phase 1 — compress less, explain more
7. Default `response_threshold_bytes` 200 → 16384 (applies to every tool,
   SkillRead included — no per-tool exemption).
8. Never compress when the placeholder would not be smaller than the original.
9. Placeholder: ~2 KB preview, line count, TOC, both tools named with their
   configured names and exact arguments.
10. Config hygiene: `context_management` fields equal to their defaults are not
    written to `settings.yaml`; setting a value equal to the default stores
    nothing. One-time migration (v28) resets values equal to a current or past
    default (response 200/4096, catalog 8192) so new defaults apply. The Tauri
    getter returns a fully-resolved view.

### Phase 2 — larger windows, cleaner output
11. IndexRead: default 200 lines, long-line split at 2000 chars, 64 KB cap,
    continuation footer with the next offset.
12. IndexSearch: default limit 5 (max 20, lenient number parsing), snippets
    3000 chars / 500 per line, cross-query dedup, consistent line numbers,
    footer with real tool + parameter names, 64 KB cap.
13. One shared preview/placeholder function (gateway, MCP-via-LLM, Tauri
    preview, demo mock).

### Phase 3 — activation
14. Fix activated tools not reaching Claude Code / "Tool not found" for listed
    tools (root cause investigation first); adjust wording if the client
    cannot refresh.

### Phase 4 — sync + final steps
15. Docs (`17-context-management.md`), demo mock, frontend defaults, TS types.
16. Local replay of recorded IndexSearch queries (private data stays in /tmp).
17. Plan review, test coverage review, bug hunt, CI-parity checks
    (clippy/fmt/test on rustup stable), commit, merge `--no-ff` to master,
    push. No release.

## Outcome

- Activation in Claude Code had the same root cause as Codex's failure: the
  MCP TS SDK opens its GET notification stream only after
  `notifications/initialized` returns 202, so `tools/list_changed` never
  reached it. The 202 fix covers both; an HTTP-level test drives the path.
  "Tool not found" for listed tools was already fixed in 0.0.150; a call to a
  tool whose server failed to start now names that server and the error.
- Offline replay of the recorded IndexSearch calls against the real payloads
  (vector search on), old vs new code, exact data only (120 calls):
  on-source hits 19% → 100%, best hit first 12/245 → 426/426, output
  2.18M → 0.61M chars, duplicated chars 766K → 0. The two recorded
  `catalog:skills` searches (source never indexed) return an error naming the
  indexed labels instead of 14K/1.6K chars of unrelated hits.
- At the 16 KB default, 51 of the 56 recorded compressed responses pass
  through whole: 50 fewer round trips, ~9% fewer chars.
- Replay found three issues, fixed before merge: minified-JSON chunks all
  claimed line 2 (now mapped to their real line, and re-serialized chunks no
  longer show line numbers that don't exist); cross-query "same as" hid
  different chunks or other parts of a chunk (now only identical snippets are
  referenced); the IndexSearch description listed `catalog:skills`/`mcp/`
  even when catalog indexing is off (now built from what the session indexes).

### Follow-ups

- Done (feat/mcp-session-id): Streamable HTTP `initialize` issues an
  `Mcp-Session-Id`; requests and the GET notification stream carrying it use
  their own gateway session (keyed `<client_id>#<id>`, so an id is useless
  with another client's token), and `DELETE` ends it. Instances sharing a
  token no longer reset each other's activated tools. Expired sessions are
  recreated lazily under the same id rather than answered with 404.
- Configs with a hand-set `response_threshold_bytes` (not a past default)
  keep it; v28 only resets values that were defaults.
- Local, outside the repo: removed the stale `[mcp_servers.localrouter]`
  entry from `~/.codex/config.toml` (401 since 09-27; the Codex client has
  MCP off, which LocalRouter's own sync would also remove); upwave
  `AGENTS.md` made client-agnostic (tool names without the client prefix,
  client-neutral deferred-tool loading, stale cross-skill path note dropped).

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

# GitHub issues #7–#10 (Add Provider templates + Coding Agents)

Four independent user-filed issues, each shipped as its **own commit** that
references its ticket, then a single `v0.0.137` release.

Commit order is chosen so the two coding-agent issues (#9 then #10) land
first — #10 depends on #9's shared binary resolver.

| # | Issue | Type | Commit scope |
|---|-------|------|--------------|
| 9 | AI CLI executables not detected despite being in PATH | bug | `lr-utils`, `lr-mcp`, `lr-coding-agents`, tauri cmds |
| 10 | Add Antigravity under Coding Agents | feat | `lr-config`, `lr-coding-agents`, TS types, demo mocks |
| 8 | Doc links + "how to find your token" in Add Provider | feat | `lr-providers` factory trait, TS types, UI |
| 7 | OpenCode Zen + Go templates in Add Provider | feat | `lr-providers` factory, registry, icons |

**Blocked on:** a concurrent agent is editing `factory.rs`,
`ProviderForm.tsx`, `providers-panel.tsx`, `tauri-commands.ts`,
`mockData.ts` for issue #12 (custom provider HTTP headers). Wait for its
commit before touching any of those files.

---

## Issue #9 — coding agent detection ignores the user's shell PATH

### Root cause

`crates/lr-coding-agents/src/manager.rs:136-157` resolves binaries with bare
`which::which(...)`, which only consults the **current process** PATH. A GUI
app (macOS `.app` from Finder/Dock, Linux `.desktop` launch) inherits a
stripped PATH that omits `~/.local/bin`, `~/.opencode/bin`, fnm/nvm node bin
dirs, `~/.cargo/bin`, `~/.bun/bin`.

A shell-PATH resolver already exists at `crates/lr-mcp/src/manager.rs:47-125`
but has two problems:

1. It is **`#[cfg(target_os = "macos")]` only** — on Linux it falls straight
   back to the process PATH (`manager.rs:71-76`). The reporter is on
   **Ubuntu 24.04 + zsh**, so this is the actual defect for them.
2. `lr-mcp` **depends on** `lr-coding-agents`, so the coding-agents crate
   cannot import it (dependency cycle).

### Fix

- **New `crates/lr-utils/src/binary.rs`** (leaf crate — no cycle):
  - `shell_env() -> HashMap<String, String>` / `shell_path() -> Option<String>`,
    cached in a `OnceLock`.
  - Resolve via `$SHELL -lic 'echo $PATH'` on **all Unix** (macOS *and* Linux),
    not macOS only. Windows keeps the process PATH.
  - Guard against a hung shell with a bounded wait, and against profile noise
    by taking the **last** non-empty output line.
  - `find_binary(name) -> Option<PathBuf>`: `which` → `which_in(shell PATH)` →
    common user-local dirs fallback (`~/.local/bin`, `~/.opencode/bin`,
    `~/.cargo/bin`, `~/.bun/bin`, `~/.deno/bin`, `/opt/homebrew/bin`,
    `/usr/local/bin`).
- `crates/lr-mcp/src/manager.rs` — delete the local copy, re-export
  `lr_utils::binary::shell_env` so existing call sites keep working.
- `crates/lr-coding-agents/src/manager.rs` — `is_agent_enabled`,
  `enabled_agents`, `detect_installed_agents` use `find_binary`.
- Honour the long-dead `CodingAgentConfig.binary_path` override
  (`lr-config/src/types.rs:1927-1929`) — it exists but is never read.
- `src-tauri/src/ui/commands_coding_agents.rs` — `binary_path` (:70) and
  `get_coding_agent_version` (:133) use the resolved path; spawn `--version`
  with the resolved PATH so a wrapper script's own `exec` still works.
- `src-tauri/src/launcher/integrations/mod.rs` — collapse its duplicate
  `find_binary` onto the shared one.

### Tests

- `shell_path()` returns something containing `/bin` on Unix.
- `find_binary("sh")` resolves.
- `find_binary` on a nonexistent name returns `None`.
- explicit `binary_path` override wins over PATH lookup.

---

## Issue #10 — add Antigravity CLI (`agy`)

Antigravity CLI is Google's successor to Gemini CLI (Gemini CLI shut down
2026-06-18). Binary is `agy`, installed by default to `~/.local/bin/agy` —
which is exactly the directory issue #9 makes visible, hence the ordering.

- `crates/lr-config/src/types.rs` — **add** `CodingAgentType::Antigravity`
  (keep `GeminiCli`; never remove a serde variant). `tool_prefix` =
  `antigravity`, `binary_name` = `agy`, `display_name` = `Antigravity`,
  in `all()`, in `supports_model_selection()` (documented `--model` flag).
  Note Gemini CLI's description as superseded.
- `crates/lr-coding-agents/src/manager.rs` — new arm in the exhaustive
  `CodingAgentType` match. `agy` is not in the executors crate, so follow the
  documented **Aider precedent** (`manager.rs:1118-1137`): Amp executor
  (plain stdin/stdout pipe) with `base_command_override: "agy"`, plus
  documented headless flags `--print`, `--output-format text`, `--model <m>`,
  and `--dangerously-skip-permissions` in auto mode.
- `crates/lr-coding-agents/src/discovery.rs` — no enumerable on-disk session
  layout, so return `Ok(vec![])` like Codex/Amp/Cursor.
- `src/types/tauri-commands.ts` — add `'antigravity'` to the
  `CodingAgentType` union.
- `website/src/components/demo/TauriMockSetup.ts` + `mockData.ts` — demo entry.

---

## Issue #8 — official source links + "how to find your token"

There is currently **no** doc/API-key link field anywhere in the provider
template pipeline. Add one end-to-end.

- `crates/lr-providers/src/factory.rs` — two new `ProviderFactory` trait
  methods with `None` defaults so no existing factory breaks:
  - `docs_url() -> Option<&'static str>` — official docs/homepage
  - `api_key_url() -> Option<&'static str>` — the exact page where the key is
    created
- `crates/lr-providers/src/registry.rs` — surface both on `ProviderTypeInfo`.
- Populate for every registered factory (~29) using each provider's real
  console URL.
- `src/types/tauri-commands.ts` + `ProviderForm.tsx` local copy — mirror
  `docsUrl` / `apiKeyUrl`.
- UI: on the configure page render "Official docs" and "Where do I find my
  API key?" links, opened with the already-imported `open()` from
  `@tauri-apps/plugin-shell` and guarded by the existing `isValidHttpUrl`
  (same pattern as `HowToConnect.tsx:369-375`).
- `website/src/components/demo/mockData.ts` — mirror on demo entries.

### Tests

- every registered factory exposes a valid `https://` `docs_url`
- every factory whose setup params include a **required** `api_key` also
  exposes an `api_key_url`

---

## Issue #7 — OpenCode Zen + OpenCode Go templates

Both are OpenAI-compatible and share one API key from
`https://opencode.ai/auth`.

| Template | `provider_type` | Base URL |
|---|---|---|
| OpenCode Zen | `opencode_zen` | `https://opencode.ai/zen/v1` |
| OpenCode Go | `opencode_go` | `https://opencode.ai/zen/go/v1` |

Follow the `KlusterAIProviderFactory` shape (`factory.rs:2076-2145`):
hardcoded base URL wrapping `OpenAICompatibleProvider`, required sensitive
`api_key`, category `ThirdParty`, `catalog_provider_id() -> None`,
`model_list_source()` default (API-only — both expose `/models`).

Touch list (from the DigitalOcean precedent):

1. `crates/lr-providers/src/factory.rs` — two factories + unit tests, and the
   cross-cutting test tables at `:3149-3250`
2. `src-tauri/src/main.rs` — imports + `register_factory` + type-string maps
3. `crates/lr-config/src/types.rs` — `ProviderType` variants + round-trip
   test tables (`:5020-5065`)
4. `src-tauri/src/ui/commands_providers.rs:484-516` — `provider_type_str_to_enum`
5. `src/components/ServiceIcon.tsx` — `ICON_MAP` + emoji fallback
6. `website/src/components/demo/mockData.ts` — demo entries

---

## Outcome

Shipped as five commits (four issues + one correction):

| Commit | Issue |
|---|---|
| `fix(coding-agents): detect CLIs installed outside the GUI process PATH` | #9 |
| `feat(coding-agents): add Antigravity (agy) support` | #10 |
| `feat(providers): link to official docs and API key pages…` | #8 |
| `feat(providers): add OpenCode Zen and OpenCode Go templates` | #7 |
| `fix(coding-agents): do not claim Antigravity session support…` | #10 follow-up |

### What the bug hunt caught

The Antigravity spawn path as first written could not have worked. Every
executor in the vibe-kanban `executors` crate hard-codes the flags of the CLI
it was written for, and `base_command_override` swaps only the base command,
not those flags (`CommandBuilder::override_base`). Routing `agy` through the
Amp executor — copying the existing Aider arm — would have produced
`agy --execute --stream-json …`, which agy rejects. The `Amp` struct also has
no `model` field, so `--model` was never passed.

`agy` is not installed on this machine, so the flags could not be verified
empirically. Rather than ship a spawn path known to be wrong, session start
now returns a self-explaining error and `supports_model_selection()` is
false. Detection and listing — the actual ask in #10 — work.

**Pre-existing bug found, not fixed here:** the Aider arm
(`lr-coding-agents/src/manager.rs`) has exactly the same defect and will
invoke `aider --execute --stream-json --no-auto-commits`. Worth its own
issue.

### Verification

- `rustup run stable cargo clippy --workspace --all-targets -- -D warnings` — clean
- `rustup run stable cargo fmt --all` — clean
- `rustup run stable cargo test --workspace` — pass
- `npx tsc --noEmit` — clean
- Every provider URL in #8/#7 checked over the network before commit
- #9's fix validated against a stripped GUI-style PATH, where `which` finds
  nothing and the login-shell probe recovers `~/.opencode/bin`,
  `~/.local/bin`, `~/.claude/local`

## Final steps (mandatory)

1. [x] Plan review — re-check implementation against this plan
2. [x] Test coverage review — cover new paths and error handling
3. [x] Bug hunt — fresh-eyes pass over each diff (found the Antigravity defect above)
4. [x] CI parity per CLAUDE.md, using the rustup **stable** toolchain
5. [x] Separate commits, each referencing its issue; push
6. [ ] `gh workflow run release.yml -f version=0.0.137`, monitor to completion
7. [ ] Comment on each ticket noting v0.0.137

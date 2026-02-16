# Fix: MCP stdio PATH resolution + command display

## Problem 1: `npx` not found in release builds
macOS `.app` bundles launched from Finder/Dock get a minimal PATH (`/usr/bin:/bin:/usr/sbin:/sbin`), missing user-installed tools like `npx`. Dev builds work because they inherit the terminal's PATH.

## Problem 2: Command/Arguments display inconsistency
The detail view shows raw `command` and `args` fields separately. New-format configs store the full command string in `command` with empty `args`, while legacy configs split them. The display doesn't normalize this.

---

## Fix 1: Shell PATH resolution

**Approach:** On app startup, resolve the user's login shell PATH by running `$SHELL -ilc 'echo $PATH'`. Store the result. When spawning stdio MCP processes, inject this PATH into the environment if not already set.

### Files to modify:

**`crates/lr-mcp/src/manager.rs`**
- Add a `shell_path: Arc<RwLock<Option<String>>>` field to `McpServerManager`
- Add `pub fn set_shell_path(&self, path: String)` method
- In `start_stdio_server()` (~line 306-322): after merging env vars, if `PATH` is not already in the env map, inject the resolved shell PATH
- In `try_spawn_command()` (~line 919): same PATH injection logic

**`src-tauri/src/main.rs`**
- After creating `McpServerManager`, resolve the shell PATH:
  ```rust
  let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
  let output = std::process::Command::new(&shell)
      .args(["-ilc", "echo $PATH"])
      .output();
  ```
- Call `mcp_manager.set_shell_path(resolved_path)` with the result
- This runs once at startup, not per-spawn

## Fix 2: Normalize command display

**File: `src/views/resources/mcp-servers-panel.tsx`**
- Lines 840-852: Change the detail view to combine `command` and `args` into a single display, same as `populateFormFromServer` already does (line 343-345):
  ```tsx
  const fullCommand = tc.args && tc.args.length > 0
    ? [tc.command, ...tc.args].join(" ")
    : tc.command
  ```
- Show single "Command" field with `fullCommand`, remove separate "Arguments" section

---

## Verification
1. `cargo test` — ensure no regressions
2. `cargo clippy` — no warnings
3. Manual: Build release, add stdio MCP server with `npx -y @modelcontextprotocol/server-everything`, verify it starts
4. Manual: Check detail view shows combined command string consistently

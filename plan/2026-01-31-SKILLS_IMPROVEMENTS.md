# Skills System Improvements Plan

## Overview
Four changes: fix critical MCP bug, add system dialog for skills, add skills wizard step, enhance skills page.

---

## 1. Fix MCP Unified Endpoint with Skills Only (Bug Fix - Priority)

**Problem A**: `set_skill_support()` takes `&mut self` but `McpGateway` is behind `Arc` — never called in production.
**Problem B**: `handle_initialize()` fails when no external MCP servers exist, even if skills are available.

### Files to modify:

**`crates/lr-mcp/src/gateway/gateway.rs`**
- Change `skill_manager: Option<Arc<SkillManager>>` → `skill_manager: OnceLock<Arc<SkillManager>>` (same for `script_executor`)
- Add `pub fn set_skill_support(&self, ...)` using `OnceLock::set()` (takes `&self`, not `&mut self`)
- Update all reads (`self.skill_manager` → `self.skill_manager.get()`)
- At line 838: before returning "All MCP servers failed to start", check if skills are available. If yes, skip to returning a minimal successful initialize result with tools capability
- At line 900: same guard — if `init_results.is_empty()` but skills are available, proceed with skills-only mode

**`src-tauri/src/main.rs`** (after line 492)
- After `app.manage(script_executor.clone())`, wire skills to gateway:
  ```rust
  if let Some(app_state) = server_manager.get_state() {
      app_state.mcp_gateway.set_skill_support(skill_manager.clone(), script_executor.clone());
  }
  ```
  (This must happen before the existing `get_state()` block at line 495, or we use a separate call)

---

## 2. System Dialog for Adding Skill Sources

**Problem**: "Add Skill Source" uses a text input. Should use native folder picker.

### Files to modify:

**`src-tauri/Cargo.toml`** — Add `tauri-plugin-dialog` dependency
**`Cargo.toml`** (workspace) — Add `tauri-plugin-dialog = "2"` to workspace deps
**`src-tauri/src/main.rs`** — Register `.plugin(tauri_plugin_dialog::init())`
**`src-tauri/capabilities/default.json`** — Add `"dialog:allow-open"` permission
**`package.json`** — Add `@tauri-apps/plugin-dialog`

**`src/views/skills/index.tsx`**
- Import `open` from `@tauri-apps/plugin-dialog`
- Replace inline text input with native folder picker dialog call
- Keep text input as a secondary "Manual path..." option for zip files / non-directory paths

---

## 3. Skills Step in Client Creation Wizard

**Problem**: No skills step in wizard. Users must configure skills after creation.

### Files to modify:

**New file: `src/components/wizard/steps/StepSkills.tsx`**
- Mirrors `ClientSkillsTab` logic: All / Specific / None access modes
- Shows skill source paths with checkboxes when "Specific" is selected
- Empty state: "No skills configured yet"

**`src/components/wizard/ClientCreationWizard.tsx`**
- Add `skillsAccessMode` and `selectedSkillPaths` to wizard state
- Insert skills step between MCP and Credentials steps
- Update step indices, titles, descriptions, `canProceed()`, `handleNext()`
- Call `set_client_skills_access` during client creation

---

## 4. Skills Page Enhancements

### 4a. Clickable source paths → open in system file explorer

**`src/views/skills/index.tsx`**
- Import `open` from `@tauri-apps/plugin-shell` (already used elsewhere)
- Make configured source path items clickable, calling `open(path)` to open in system file explorer

### 4b. Skill detail: link to open skill directory

**`src/views/skills/index.tsx`**
- Add "Open folder" button next to source path in skill detail panel
- Calls `open(selectedSkillInfo.source_path)` — for files, open containing folder

### 4c. Skill detail: list files with collapsible previews

**`src-tauri/src/ui/commands.rs`**
- New `get_skill_files` command returning file list per skill (scripts, references, assets) with content previews (truncated to ~500 chars)
- Register in `main.rs` invoke_handler

**`src/views/skills/index.tsx`**
- Fetch files when skill is selected
- Display collapsible sections per category (Scripts, References, Assets)
- Each file shows name + collapsed content preview (`<pre>` block)

---

## Verification

1. `cargo test && cargo clippy && cargo fmt`
2. `cargo tauri dev` — verify:
   - Skills page: "Add Skill Source" opens native folder picker
   - Skills page: source paths are clickable → opens file explorer
   - Skills page: selecting a skill shows files with previews
   - Client wizard: skills step appears between MCP and Credentials
   - MCP endpoint: client with skills but no MCP servers gets successful initialize with skill tools

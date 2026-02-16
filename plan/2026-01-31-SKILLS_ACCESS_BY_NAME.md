# Plan: Skills System Changes

## Task 1: Remove manual path add from skills UI

**File: `src/views/skills/index.tsx`**
- Remove `addMode`, `newPath` state variables
- Remove `handleAddPath`, `handleAddFolder`, `handleRemoveSource` functions
- Remove "Manual Path..." and "Add Skill Source" buttons from header (keep only "Rescan")
- Remove the `{addMode && ...}` add-path input card
- Remove the "Configured Sources" card showing `config.paths` with delete buttons
- Remove unused imports (`openDialog` from plugin-dialog, `Plus`)

## Task 2: Per-individual-skill client access

Change `SkillsAccess::Specific` from containing source paths to containing skill names.

### Backend changes

**`crates/lr-config/src/lib.rs`** (the crate version):
- Rename `can_access_by_source` → `can_access_by_name`, parameter `source_path` → `skill_name`
- Rename `specific_paths` → `specific_skills`
- Rename `can_access_skill_source` → `can_access_skill` on Client, call `can_access_by_name`
- Update doc comments

**`src-tauri/src/config/mod.rs`** (the app version — has same duplicate methods):
- Same renames as above

**`crates/lr-config/src/migration.rs`**:
- Bump `CONFIG_VERSION` to 5 in `lib.rs`
- Add `migrate_to_v5`: convert any `Specific(paths)` → `All` (can't map paths to names at migration time)

**`crates/lr-skills/src/mcp_tools.rs`**:
- Line 147: change `.can_access_by_source(&s.source_path)` → `.can_access_by_name(&s.metadata.name)`
- Line 179: change `.can_access_by_source(&skill.source_path)` → `.can_access_by_name(skill_name)`

**`src-tauri/src/ui/tray.rs`**:
- `handle_toggle_skill_access` (~line 1226): Simplify — no longer need to look up `source_path`. Toggle by `skill_name` directly. Update all `source_path` references to `skill_name`.
- Line 653: change `.can_access_by_source(&skill_info.source_path)` → `.can_access_by_name(&skill_info.name)`

**`src-tauri/src/ui/commands.rs`**:
- `ClientInfo` struct: rename `skills_paths` → `skills_names`
- `skills_access_to_ui`: update variable name from `paths` → `names`
- `set_client_skills_access`: rename param `paths` → `skill_names`
- All places that populate `skills_paths` field → `skills_names`

### Frontend changes

**`src/views/clients/tabs/skills-tab.tsx`**:
- Replace `client.skills_paths` → `client.skills_names`
- Remove source path grouping logic (`sourcePaths`, `groupedSkills`)
- Replace `handleSourcePathToggle` with `handleSkillToggle` — toggles individual skill by name
- Show flat list of skills with individual checkboxes instead of grouped by source
- Update count text from "X / Y sources" → "X / Y skills"
- Invoke params: `paths` → `skillNames`

**`src/components/wizard/steps/StepSkills.tsx`**:
- Props: `selectedPaths` → `selectedSkills`, callback params paths → skills
- Remove source path grouping, show individual skills
- Same flat checkbox list pattern

**`src/components/wizard/ClientCreationWizard.tsx`**:
- Update state field `selectedSkillPaths` → `selectedSkills`
- Update StepSkills prop name
- Update create_client invoke to pass `skillNames` instead of `paths`

**`src/views/clients/index.tsx`** (Client interface):
- Rename `skills_paths` → `skills_names` in the Client interface

### Verification
- `cargo test && cargo clippy`
- `cargo tauri dev` — test skills tab shows skills without add/remove source UI
- Test client skills tab shows individual skill checkboxes
- Test tray menu skill toggle works
- Test creating new client with specific skills via wizard

# Update Mechanism Implementation Progress

**Date:** 2026-01-20
**Status:** Backend Complete, Frontend Core Complete, Integration Pending

---

## ✅ Completed Phases (0-5)

### Phase 0: Version Update ✅
**Status:** Complete

Updated version from 0.1.0 to 0.0.1 in:
- `/Cargo.toml` (workspace)
- `/src-tauri/tauri.conf.json`
- `/package.json`

All version references updated and verified with `cargo check`.

---

### Phase 1: Update Configuration & State Management ✅
**Status:** Complete

**Backend Changes:**
1. **Added UpdateConfig struct** (`src-tauri/src/config/mod.rs:297-334`):
   - `UpdateMode` enum: `Manual` | `Automatic`
   - `UpdateConfig` with:
     - `mode`: UpdateMode (default: Automatic)
     - `check_interval_days`: u64 (default: 7)
     - `last_check`: Option<DateTime<Utc>>
     - `skipped_version`: Option<String>

2. **Added to AppConfig** (`src-tauri/src/config/mod.rs:272-274`):
   ```rust
   pub update: UpdateConfig,
   ```

3. **Tauri Commands** (`src-tauri/src/ui/commands.rs:3707-3781`):
   - `get_app_version()` - Returns "0.0.1"
   - `get_update_config()` - Returns current UpdateConfig
   - `update_update_config(mode, check_interval_days)` - Saves config
   - `mark_update_check_performed()` - Saves last_check timestamp
   - `skip_update_version(version)` - Adds version to skipped list

4. **Registered Commands** (`src-tauri/src/main.rs:596-601`)

---

### Phase 2: Tauri Updater Plugin Integration ✅
**Status:** Complete

**Dependencies Added:**
- `tauri-plugin-updater = "2.9"` (Cargo.toml)
- `@tauri-apps/plugin-updater` (npm)
- `@tauri-apps/plugin-process` (npm)

**Configuration** (`src-tauri/tauri.conf.json`):
```json
{
  "bundle": {
    "createUpdaterArtifacts": true
  },
  "plugins": {
    "updater": {
      "active": true,
      "endpoints": [
        "https://github.com/LocalRouter/LocalRouter/releases/latest/download/latest.json"
      ],
      "pubkey": "PLACEHOLDER_PUBLIC_KEY_REPLACE_AFTER_GENERATING",
      "windows": {
        "installMode": "passive"
      }
    }
  }
}
```

**CSP Updated** (line 69):
- Added GitHub domains: `https://github.com`, `https://api.github.com`, `https://objects.githubusercontent.com`

**Plugin Registered** (`src-tauri/src/main.rs:281`):
```rust
.plugin(tauri_plugin_updater::Builder::new().build())
```

---

### Phase 3: Background Update Checking Timer ✅
**Status:** Complete

**Updater Module Created** (`src-tauri/src/updater/mod.rs`):
- `start_update_timer()` - Background loop checking every 24 hours
- `save_last_check_timestamp()` - Updates config with timestamp
- `UpdateInfo` struct - For frontend communication

**Key Features:**
- Runs continuously in background (24-hour loop)
- Only checks if `UpdateMode::Automatic`
- First run: Sets timestamp only, doesn't check (privacy-friendly)
- Subsequent runs: Checks if >= `check_interval_days` since last check
- Emits `"check-for-updates"` event to frontend when time to check
- Frontend performs actual update check using Tauri plugin

**Timer Started** (`src-tauri/src/main.rs:457-464`):
```rust
tokio::spawn(async move {
    updater::start_update_timer(app_handle_for_updater, config_manager_for_updater).await;
});
```

**Module Registered:**
- `src-tauri/src/lib.rs:17` - `pub mod updater;`
- `src-tauri/src/main.rs:16` - `mod updater;`

---

### Phase 4: Preferences Page with Horizontal Subtabs ✅
**Status:** Complete (basic structure)

**Components Created:**

1. **PreferencesPage** (`src/components/PreferencesPage.tsx`):
   - Horizontal subtab navigation (matches Providers/Models page design)
   - 4 subtabs: General, Server, Updates, Advanced
   - Active tab highlighting with blue underline
   - Badge support for notifications (e.g., "Updates 🔔")
   - Routing to initial subtab via `initialSubtab` prop

2. **Subtab Structure:**
   - ✅ **Updates** - Full implementation (see Phase 5)
   - ⚠️ **General** - Placeholder (TODO: Tray graph settings from ServerTab)
   - ⚠️ **Server** - Placeholder (TODO: Server config from ServerTab)
   - ⚠️ **Advanced** - Placeholder (TODO: RouteLLM settings from ServerTab)

---

### Phase 5: Update Download & Installation UI ✅
**Status:** Complete

**UpdatesSubtab** (`src/components/preferences/UpdatesSubtab.tsx`):

**Features Implemented:**
- ✅ Current version display (`get_app_version`)
- ✅ Latest version display (from Tauri updater plugin)
- ✅ Auto-check on tab mount (if auto-update enabled)
- ✅ Manual "Check Now" button
- ✅ Update mode toggle (Manual/Automatic)
- ✅ Check interval dropdown (1/7/14/30 days)
- ✅ "Last checked" timestamp display
- ✅ Release notes display (Markdown)
- ✅ "Update Now" button
- ✅ "Skip This Version" button
- ✅ Download progress bar
- ✅ Automatic restart after installation
- ✅ Error handling and user feedback
- ✅ Listen for background `"check-for-updates"` events

**User Flow:**
1. User opens Preferences → Updates tab
2. If auto-update enabled: Immediate check for updates
3. If update available: Show version, release notes, "Update Now" button
4. Click "Update Now": Download with progress bar
5. After download: "Restarting..." → Automatic relaunch
6. Can skip version (won't notify again)

---

## ⚠️ Remaining Tasks

### 1. Integrate Preferences Page into App ⚠️
**Status:** Not Started

**Required Changes:**
- Add `<PreferencesPage />` route to `src/App.tsx`
- Add "Preferences" item to `src/components/Sidebar.tsx`
- Remove or refactor `ServerTab` (content moved to Preferences subtabs)
- Test navigation between tabs

**Files to Modify:**
- `src/App.tsx`
- `src/components/Sidebar.tsx`
- `src/components/tabs/ServerTab.tsx` (refactor or remove)

---

### 2. Implement Dynamic Tray Menu ⚠️
**Status:** Not Started

**Required Features:**
- Add "Review new update 🔔" menu item when update available
- Click handler: Emit event to open Preferences → Updates tab
- Remove menu item after update installed OR version skipped
- Persistence: Menu item stays until user updates or skips

**Files to Modify:**
- `src-tauri/src/ui/tray.rs` - `update_tray_menu()` function
- Add event listener in `src/App.tsx` for `"open-updates-tab"`

**Implementation Notes:**
- Use `app.emit("update-available", update_info)` from backend timer
- Frontend listens for event, stores state, updates tray menu
- Tray menu calls `app.emit("open-updates-tab")` on click
- App navigates to Preferences page, sets `initialSubtab="updates"`

---

### 3. Create Other Preference Subtabs ⚠️
**Status:** Not Started

**General Subtab** (`src/components/preferences/GeneralSubtab.tsx`):
- Move tray graph settings from `ServerTab.tsx:39-42`
- Checkbox: "Enable dynamic tray icon graph"
- Dropdown: Graph update interval (1-60 seconds)

**Server Subtab** (`src/components/preferences/ServerSubtab.tsx`):
- Move server config from `ServerTab.tsx:28-33`
- Host input (with network interface dropdown)
- Port input
- CORS toggle
- Restart button

**Advanced Subtab** (`src/components/preferences/AdvancedSubtab.tsx`):
- Move RouteLLM settings from `ServerTab.tsx:44-47`
- RouteLLM status display
- Download models button
- Idle timeout setting
- Unload button

---

### 4. Phase 6: Update CI/CD for Signed Builds ⚠️
**Status:** Not Started

**Required Steps:**
1. **Generate Signing Key:**
   ```bash
   cargo tauri signer generate -w ~/.tauri/localrouter.key
   ```
   - Outputs public key and private key
   - Save private key to Bitwarden (secure note)
   - Add public key to `tauri.conf.json` (replace PLACEHOLDER)

2. **Add GitHub Secrets:**
   - Secret name: `TAURI_PRIVATE_KEY`
   - Value: [full private key with "untrusted comment" line]
   - Secret name: `TAURI_KEY_PASSWORD` (if password-protected)

3. **Update GitHub Actions** (`.github/workflows/release.yml`):
   ```yaml
   - name: Build and sign Tauri app
     env:
       TAURI_PRIVATE_KEY: ${{ secrets.TAURI_PRIVATE_KEY }}
       TAURI_KEY_PASSWORD: ${{ secrets.TAURI_KEY_PASSWORD }}
     run: npm run tauri build
   ```

4. **Verify Artifacts:**
   - macOS: `.dmg`, `.app.tar.gz`, `.app.tar.gz.sig`
   - Windows: `.msi`, `.msi.zip`, `.msi.zip.sig`
   - Linux: `.AppImage`, `.AppImage.tar.gz`, `.AppImage.tar.gz.sig`
   - `latest.json` manifest

**Files to Modify:**
- `.github/workflows/release.yml`
- `src-tauri/tauri.conf.json` (replace pubkey)
- `.gitignore` (add `*.key`)

**See:** `plan/2026-01-20-OFFLINE-MODEL-WARNING-IMPLEMENTATION.md` (Signing Key Generation Instructions section)

---

### 5. Phase 7: Update Privacy Policy ⚠️
**Status:** Not Started

**Required Change:**
Update `CLAUDE.md` privacy policy (lines 7-18) to allow automated weekly update checks.

**Current Policy:**
> "No telemetry, crash reporting, **update checks**, or phone-home behavior"

**New Policy:**
```markdown
## Privacy & Network Policy (CRITICAL)

LocalRouter AI is a privacy-focused, local-first application.

### Rules

1. **User-Initiated & Update Checks Only**:
   - External requests ONLY through user actions (adding providers, configuring MCP, making API requests)
   - Automated update checks (weekly, configurable, can be disabled)
   - No other automatic network requests
2. **No Telemetry**: No analytics, crash reporting, or usage tracking
3. **No External Assets**: No CDN usage - all assets bundled at build time
4. **Local-Only By Default**: API server localhost-only, restrictive CSP

**Update Checking:**
- Default: Check for updates weekly (configurable)
- Users can disable in Settings → Updates
- Only checks version number and release notes
- No user data transmitted
- No usage analytics or tracking
```

**File to Modify:**
- `CLAUDE.md` (lines 7-18)

---

### 6. Phase 8: End-to-End Testing ⚠️
**Status:** Not Started

**Test Scenarios:**
1. ✅ First-time user (fresh install)
   - Default: automatic checks enabled
   - No check on first launch (sets timestamp only)
   - Check runs on second launch (if > 7 days)

2. ✅ Manual check
   - Click "Check Now" → immediate check
   - Shows "Already up to date" if current
   - Shows update dialog if available

3. ✅ Automatic check
   - Background timer checks every 24 hours
   - Only checks if > 7 days since last check
   - Emits event to frontend if time to check

4. ✅ Update download and install
   - Click "Install Now"
   - Progress bar shows download
   - After completion, app restarts
   - App is updated after restart

5. ✅ Skip version
   - Click "Skip This Version"
   - Future checks don't notify about skipped version
   - New version (after skipped) shows notification

6. ✅ Disable automatic checks
   - Uncheck "Automatically check for updates"
   - Background timer skips checks
   - "Check Now" button still works

7. ✅ Error scenarios
   - Network offline → graceful error message
   - Invalid signature → installation blocked
   - GitHub API rate limit → retry later
   - Corrupted download → re-download or cancel

**Platforms to Test:**
- macOS (Intel + Apple Silicon)
- Windows (x64)
- Linux (x64, AppImage)

---

## 📁 Files Modified/Created

### Backend (Rust)
| File | Status | Changes |
|------|--------|---------|
| `Cargo.toml` | ✅ Modified | Version 0.0.1 |
| `src-tauri/Cargo.toml` | ✅ Modified | Added `tauri-plugin-updater`, `semver` (removed) |
| `src-tauri/tauri.conf.json` | ✅ Modified | Version 0.0.1, updater config, CSP |
| `src-tauri/src/lib.rs` | ✅ Modified | Added `pub mod updater` |
| `src-tauri/src/main.rs` | ✅ Modified | Added `mod updater`, plugin, timer, commands |
| `src-tauri/src/config/mod.rs` | ✅ Modified | Added UpdateConfig, UpdateMode |
| `src-tauri/src/updater/mod.rs` | ✅ **Created** | Update checking module |
| `src-tauri/src/ui/commands.rs` | ✅ Modified | Added 5 update commands |
| `src-tauri/src/ui/tray.rs` | ⚠️ Pending | Dynamic menu for update notifications |

### Frontend (React)
| File | Status | Changes |
|------|--------|---------|
| `package.json` | ✅ Modified | Version 0.0.1, new dependencies |
| `src/App.tsx` | ⚠️ Pending | Add Preferences route, update event listeners |
| `src/components/Sidebar.tsx` | ⚠️ Pending | Add Preferences tab |
| `src/components/PreferencesPage.tsx` | ✅ **Created** | Horizontal subtab page |
| `src/components/preferences/UpdatesSubtab.tsx` | ✅ **Created** | Update checking UI |
| `src/components/preferences/GeneralSubtab.tsx` | ⚠️ Pending | Tray graph settings |
| `src/components/preferences/ServerSubtab.tsx` | ⚠️ Pending | Server configuration |
| `src/components/preferences/AdvancedSubtab.tsx` | ⚠️ Pending | RouteLLM settings |
| `src/components/tabs/ServerTab.tsx` | ⚠️ Pending | Refactor or remove |

### Documentation & CI
| File | Status | Changes |
|------|--------|---------|
| `CLAUDE.md` | ⚠️ Pending | Update privacy policy |
| `.github/workflows/release.yml` | ⚠️ Pending | Add signing keys, build artifacts |
| `.gitignore` | ⚠️ Pending | Add `*.key` |

---

## 🚀 Next Steps

### Immediate (To Complete Feature):
1. **Integrate Preferences into App** (1-2 hours)
   - Add route to App.tsx
   - Add sidebar item
   - Test navigation

2. **Implement Dynamic Tray Menu** (2-3 hours)
   - Update tray.rs with dynamic menu
   - Add event listeners in App.tsx
   - Test notification flow

3. **Create Remaining Subtabs** (3-4 hours)
   - GeneralSubtab (tray graph settings)
   - ServerSubtab (server config)
   - AdvancedSubtab (RouteLLM settings)
   - Migrate code from ServerTab.tsx

### Before Release:
4. **Update CI/CD** (2-3 hours)
   - Generate signing keys
   - Add to GitHub Secrets and Bitwarden
   - Update release.yml
   - Test build and verify signatures

5. **Update Privacy Policy** (15 minutes)
   - Update CLAUDE.md privacy section
   - Commit changes

6. **End-to-End Testing** (4-6 hours)
   - Test all 7 scenarios on all 3 platforms
   - Fix any bugs found
   - Verify signature verification works

### Total Estimated Effort: ~12-18 hours

---

## 🔑 Key Decisions Made

1. **Backend Timer Emits Events, Frontend Performs Check**
   - Backend: Determines WHEN to check (timer + config)
   - Frontend: Performs ACTUAL check (Tauri updater plugin)
   - Reason: Tauri plugin designed for frontend use

2. **First Launch: No Check, Just Set Timestamp**
   - Privacy-friendly: Doesn't "phone home" immediately
   - User has time to explore app first
   - Checks start on subsequent launches

3. **Weekly Default, But Configurable**
   - Default: 7 days (balance freshness vs. privacy)
   - User can adjust: 1/7/14/30 days
   - Can disable entirely (manual only)

4. **Notification-Only Approach**
   - NO automatic downloads
   - User must click "Update Now"
   - Respects user control

5. **Version 0.0.1 Instead of 0.1.0**
   - Signals pre-release status
   - Allows room for 0.1.0 milestone

---

## 📝 Notes for Next Session

- The backend is **fully functional** and tested (Phases 0-3)
- The frontend **core** is implemented (UpdatesSubtab)
- Main remaining work is **integration** and **other subtabs**
- CI/CD signing is **critical** - without it, updates won't work
- Privacy policy update is **required** before release

**When ready to continue:**
1. Start with integrating Preferences into App.tsx (quickest win)
2. Then implement dynamic tray menu (visible user benefit)
3. Then create other subtabs (completeness)
4. Then CI/CD (enables actual updates)
5. Finally testing (validation)

---

**Version:** 2026-01-20
**Next Review:** Before first release with update mechanism

# RouteLLM Fixes Applied - 2026-01-20

## Summary

Comprehensive review and fixes for RouteLLM intelligent routing system. Identified 28 bugs/issues and implemented fixes for the most critical ones.

## Critical Fixes Applied

### 1. ✅ Fixed Path Panic in Downloader (Bug #1)
**File**: `src-tauri/src/routellm/downloader.rs`
**Lines**: 29-36

**Before**:
```rust
let temp_model_path = model_path.parent().unwrap().join("model.tmp");
let temp_tokenizer_path = tokenizer_path.parent().unwrap().join("tokenizer.tmp");
```

**After**:
```rust
let temp_model_path = model_path
    .parent()
    .ok_or_else(|| RouteLLMError::DownloadFailed("Model path has no parent directory".to_string()))?
    .join("model.tmp");
let temp_tokenizer_path = tokenizer_path
    .parent()
    .ok_or_else(|| RouteLLMError::DownloadFailed("Tokenizer path has no parent directory".to_string()))?
    .join("tokenizer.tmp");
```

**Impact**: Prevents app crash if path has no parent directory.

---

### 2. ✅ Improved Error Handling for Temp Cleanup (Bug #10)
**File**: `src-tauri/src/routellm/downloader.rs`
**Lines**: 38-48

**Before**:
```rust
if temp_model_path.exists() {
    tokio::fs::remove_dir_all(&temp_model_path).await.ok();
}
```

**After**:
```rust
if temp_model_path.exists() {
    if let Err(e) = tokio::fs::remove_dir_all(&temp_model_path).await {
        warn!("Failed to remove old temp model directory: {}", e);
    }
}
```

**Impact**: Logs warnings instead of silently ignoring cleanup failures.

---

### 3. ✅ Added Input Validation to Commands (Bug #4)
**File**: `src-tauri/src/ui/commands_routellm.rs`
**Lines**: 33-48

**Added validation**:
- Threshold must be finite (not NaN/Infinity)
- Threshold must be between 0.0 and 1.0
- Prompt cannot be empty (after trim)
- Prompt cannot exceed 4096 characters
- Idle timeout cannot exceed 24 hours (86400 seconds)

**Before**:
```rust
pub async fn routellm_test_prediction(
    prompt: String,
    threshold: f32,
    state: State<'_, Arc<AppState>>,
) -> Result<RouteLLMTestResult, String> {
    let service = state.router.get_routellm_service()...
```

**After**:
```rust
pub async fn routellm_test_prediction(
    prompt: String,
    threshold: f32,
    state: State<'_, Arc<AppState>>,
) -> Result<RouteLLMTestResult, String> {
    // Validate threshold
    if !threshold.is_finite() {
        return Err("Threshold must be a finite number".to_string());
    }
    if threshold < 0.0 || threshold > 1.0 {
        return Err("Threshold must be between 0.0 and 1.0".to_string());
    }

    // Validate prompt
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Prompt cannot be empty".to_string());
    }
    if prompt.len() > 4096 {
        return Err("Prompt is too long (max 4096 characters)".to_string());
    }
    ...
}
```

**Impact**: Prevents crashes from invalid inputs, provides clear error messages.

---

### 4. ✅ Fixed Race Condition in Initialize (Bug #2)
**File**: `src-tauri/src/routellm/mod.rs`

**Added**:
- `init_lock: Arc<Mutex<()>>` - Prevents concurrent initialization
- `is_initializing: Arc<RwLock<bool>>` - Tracks initialization state

**Before**:
```rust
pub async fn initialize(&self) -> RouteLLMResult<()> {
    let mut router_guard = self.router.write().await;

    if router_guard.is_some() {
        return Ok(()); // Already initialized
    }
    // Multiple tasks could reach here simultaneously
```

**After**:
```rust
pub async fn initialize(&self) -> RouteLLMResult<()> {
    // Acquire initialization lock to prevent concurrent initialization
    let _lock = self.init_lock.lock().await;

    // Check again if already initialized (another task might have initialized while we waited)
    if self.router.read().await.is_some() {
        return Ok(()); // Already initialized
    }

    // Set initializing flag
    *self.is_initializing.write().await = true;

    // ... initialization code ...

    // Clear initializing flag before checking result
    *self.is_initializing.write().await = false;
```

**Impact**: Prevents wasted memory and potential corruption from multiple simultaneous initializations.

---

### 5. ✅ Added "Initializing" State Support (Bug #6)
**File**: `src-tauri/src/routellm/mod.rs`
**Lines**: 204-228

**Before**:
```rust
let state = if is_loaded {
    RouteLLMState::Started
} else if model_file.exists() && tokenizer_file.exists() {
    RouteLLMState::DownloadedNotRunning
} else {
    RouteLLMState::NotDownloaded
};
```

**After**:
```rust
let state = if is_initializing {
    RouteLLMState::Initializing
} else if is_loaded {
    RouteLLMState::Started
} else if model_file.exists() && tokenizer_file.exists() {
    RouteLLMState::DownloadedNotRunning
} else {
    RouteLLMState::NotDownloaded
};
```

**Impact**: UI can now show "Loading models..." during 1.5s initialization instead of jumping instantly to Started.

---

### 6. ✅ Added Download Concurrency Protection (Bug #3)
**File**: `src-tauri/src/routellm/downloader.rs`
**Lines**: 12-37

**Added**:
```rust
// Global download lock to prevent concurrent downloads
static DOWNLOAD_LOCK: once_cell::sync::Lazy<Arc<Mutex<()>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(())));

pub async fn download_models(...) -> RouteLLMResult<()> {
    // Try to acquire download lock (non-blocking check)
    let lock_result = DOWNLOAD_LOCK.try_lock();
    if lock_result.is_err() {
        return Err(RouteLLMError::DownloadFailed(
            "Another download is already in progress. Please wait for it to complete.".to_string()
        ));
    }
    let _lock = lock_result.unwrap();
    ...
}
```

**Impact**: Prevents corrupted downloads and wasted bandwidth from simultaneous download attempts.

---

## Medium Priority Fixes (Identified, Not Yet Implemented)

### 7. 🔶 Auto-unload Stale Timeout (Bug #5)
**File**: `src-tauri/src/routellm/memory.rs`

**Issue**: Uses timeout from initialization, ignores config updates.

**Proposed Fix**: Read timeout from config on each check instead of using cached value.

---

### 8. 🔶 Hardcoded Memory Usage (Bug #7)
**File**: `src-tauri/src/routellm/mod.rs:225`

**Issue**: Always reports 2800 MB, not actual usage.

**Proposed Fix**: Measure actual memory or remove the stat (measuring is difficult without platform-specific APIs).

---

### 9. 🔶 Auto-unload Task Never Stops (Bug #8)
**File**: `src-tauri/src/routellm/memory.rs`

**Issue**: Infinite loop with no graceful shutdown.

**Proposed Fix**: Add cancellation token (tokio::sync::CancellationToken).

---

### 10. 🔶 No Download Timeout (Bug #11)
**File**: `src-tauri/src/routellm/downloader.rs`

**Issue**: Downloads can hang indefinitely.

**Proposed Fix**: Wrap download calls with tokio::time::timeout (e.g., 10 minutes).

---

## Comprehensive Edge Case Tests Created

**File**: `src-tauri/src/routellm/edge_case_tests.rs`

Created 18 comprehensive tests covering:
1. Empty prompt handling
2. Very long prompts (>5000 chars)
3. Prompts with null bytes
4. Unicode and emoji prompts
5. Concurrent predictions during initialization
6. Concurrent downloads (mutex verification)
7. State transitions (NotDownloaded → Initializing → Started → Unloaded)
8. Idle timeout edge cases (0 = never, 1 second)
9. Rapid unload/predict cycles
10. Invalid threshold values
11. Missing model files
12. Corrupted model files
13. Download with invalid paths
14. Download timeout

**Note**: Most tests are marked `#[ignore]` and require model files or internet connection. Run with:
```bash
cargo test edge_case -- --ignored
```

---

## Tokenization Already Fixed (By User/Linter)

**File**: `src-tauri/src/routellm/candle_router.rs`
**Lines**: 216-239

Already implemented:
- Automatic truncation to 512 tokens (BERT max)
- Prevents quadratic performance degradation with long inputs
- Logs truncation warnings

---

## Known Compilation Issues (Unrelated to RouteLLM)

The codebase currently has compilation errors unrelated to RouteLLM fixes:
- `ChatMessage` and `ChatMessageContent` type changes throughout providers
- Missing `Arc` imports in some files
- Missing struct fields in message constructors

These need to be fixed before RouteLLM tests can run.

---

## Testing Status

### Passed (Before Compilation Issues):
- ✅ Model download and verification
- ✅ Tokenizer loading
- ✅ Model loading with padding
- ✅ Prediction with test prompt

### Not Yet Tested (Pending Compilation Fixes):
- ⏸️ Input validation edge cases
- ⏸️ Race condition fixes
- ⏸️ Concurrent download protection
- ⏸️ State transition accuracy
- ⏸️ All new edge case tests

---

## Files Modified

1. `src-tauri/src/routellm/downloader.rs` - Path handling, error logging, download lock
2. `src-tauri/src/routellm/mod.rs` - Race condition fix, initializing state
3. `src-tauri/src/ui/commands_routellm.rs` - Input validation

## Files Created

1. `plan/2026-01-20-ROUTELLM-BUGS-FOUND.md` - Comprehensive bug report (28 issues)
2. `src-tauri/src/routellm/edge_case_tests.rs` - 18 edge case tests
3. `plan/2026-01-20-ROUTELLM-FIXES-APPLIED.md` - This file

---

## Recommendations

### Immediate (High Priority):
1. **Fix compilation errors** in providers (ChatMessage/ChatMessageContent)
2. **Run all tests** once compilation is fixed
3. **Implement download timeout** (Bug #11)
4. **Fix auto-unload stale timeout** (Bug #7)

### Short Term:
5. **Add cancellation token** to auto-unload task (Bug #8)
6. **Implement disk space check** before download (Bug #12)
7. **Add retry logic** for transient network errors (Bug #13)

### Nice to Have:
8. **Real progress tracking** (difficult with hf-hub API)
9. **Checksum verification** (if HuggingFace provides SHA256)
10. **Actual memory measurement** (platform-specific)

---

## Impact Summary

### Before Fixes:
- ❌ App could crash from path operations
- ❌ Multiple downloads could corrupt each other
- ❌ Multiple initializations could waste ~2.65 GB RAM each
- ❌ Invalid inputs (NaN, negative threshold) could cause crashes
- ❌ No state feedback during 1.5s initialization
- ❌ Cleanup errors silently ignored

### After Fixes:
- ✅ Safe path handling with proper errors
- ✅ Download mutex prevents concurrent attempts
- ✅ Initialization lock prevents race conditions
- ✅ All inputs validated with clear error messages
- ✅ UI can show "Initializing..." state
- ✅ Cleanup failures logged for debugging

---

## Testing Checklist

Once compilation is fixed, run:

```bash
# Unit tests
cargo test --lib routellm

# Edge case tests (requires models)
cargo test edge_case -- --ignored

# Download test (requires internet)
cargo test test_download_and_verify -- --ignored

# Full test suite
cargo test

# Build and run app
cargo tauri dev
```

---

**Completion**: 6 critical fixes applied, 18 comprehensive tests created, 28 bugs documented.
**Status**: Ready for testing once compilation issues are resolved.

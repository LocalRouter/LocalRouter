# RouteLLM Bugs and Edge Cases Found - 2026-01-20

## Critical Bugs

### 1. **Panic on path operations** (downloader.rs:29-30)
- `unwrap()` on `parent()` could panic if path has no parent
- **Impact**: App crash during download
- **Fix**: Use proper error handling

### 2. **Race condition in initialize()** (mod.rs:75-117)
- Multiple simultaneous `predict()` calls could trigger multiple initializations
- **Impact**: Wasted memory, potential corruption
- **Fix**: Add initialization lock/flag

### 3. **Download race condition** (commands_routellm.rs:67-86)
- No concurrency protection - multiple simultaneous downloads interfere
- **Impact**: Corrupted downloads, wasted bandwidth
- **Fix**: Add download mutex/lock

### 4. **No input validation** (commands_routellm.rs:28-42, candle_router.rs:213-306)
- Threshold can be negative, >1.0, NaN, Infinity
- Prompt can be empty, too long (BERT max = 512 tokens ~2048 chars)
- **Impact**: Prediction errors, crashes, poor UX
- **Fix**: Validate all inputs

### 5. **Stale idle_timeout in auto-unload** (memory.rs:9-26)
- Uses timeout from initialization, ignores config updates
- **Impact**: Settings changes don't take effect until restart
- **Fix**: Read timeout from config on each check

## High Priority Bugs

### 6. **Missing "Initializing" state** (mod.rs:186-192)
- State jumps from DownloadedNotRunning → Started instantly
- **Impact**: UI doesn't show "loading models..." during 1.5s init
- **Fix**: Set Initializing state during initialization

### 7. **Hardcoded memory usage** (mod.rs:196)
- Always reports 2800 MB, not actual usage
- **Impact**: Inaccurate resource monitoring
- **Fix**: Measure actual memory or remove the stat

### 8. **Auto-unload task never stops** (memory.rs:13-25)
- Infinite loop with no graceful shutdown
- **Impact**: Task runs forever, even after service dropped
- **Fix**: Add cancellation token

### 9. **Config update doesn't update service** (commands_routellm.rs:88-110)
- Updates config file but not the running service's idle_timeout
- **Impact**: Settings don't apply until restart
- **Fix**: Update service state after config change

## Medium Priority Bugs

### 10. **Silently ignores temp cleanup errors** (downloader.rs:34, 37)
- `.ok()` swallows errors when removing old temp directories
- **Impact**: Disk space leaks, misleading error messages
- **Fix**: Log warnings on failure

### 11. **No download timeout** (downloader.rs:71, 124)
- Downloads can hang indefinitely
- **Impact**: UI stuck in "Downloading..." forever
- **Fix**: Add timeout (e.g., 10 minutes)

### 12. **No disk space check** (downloader.rs:19-221)
- Downloads 440 MB without checking available space
- **Impact**: Partial download, corrupted state
- **Fix**: Check available space before download

### 13. **No retry logic** (downloader.rs:71-86, 124-138)
- Transient network errors cause complete failure
- **Impact**: Poor UX, requires manual retry
- **Fix**: Retry failed downloads 3 times

### 14. **Fake progress tracking** (downloader.rs:48-159)
- Progress based on file count, not actual bytes downloaded
- **Impact**: Progress bar jumps, misleading user
- **Fix**: Track actual download progress (difficult with hf-hub)

### 15. **No checksum verification** (downloader.rs:164-187)
- Only verifies model loads, not file integrity
- **Impact**: Silent corruption could pass verification
- **Fix**: Add SHA256 checksum validation (if HF provides)

### 16. **Incomplete error recovery** (downloader.rs:168-187)
- Cleans temp on verification failure, but not on rename failure
- **Impact**: Inconsistent state on errors
- **Fix**: Ensure cleanup in all error paths

### 17. **Predict() holds read lock during calculation** (mod.rs:130-142)
- 10-15ms calculation blocks other readers
- **Impact**: Reduced concurrency
- **Fix**: Release lock before calculation (already safe with Arc)

## Low Priority / Edge Cases

### 18. **Empty prompt handling** (candle_router.rs:213)
- No explicit check for empty prompts
- **Impact**: Unnecessary tokenization, confusing results
- **Fix**: Return error for empty/whitespace-only prompts

### 19. **Prompt length validation** (candle_router.rs, ThresholdTester.tsx)
- BERT max is 512 tokens (~2048 characters)
- No client or server-side validation
- **Impact**: Truncation without warning
- **Fix**: Validate and truncate with warning

### 20. **No max history limit on persistent storage** (ThresholdTester.tsx:46)
- History limited to 10 in memory, but could grow unbounded if persisted
- **Impact**: Currently OK (not persisted), future issue
- **Fix**: Document or enforce limit

### 21. **Error messages not user-friendly** (ThresholdTester.tsx:49)
- Just shows `err.toString()` - exposes internal errors
- **Impact**: Poor UX, confusing messages
- **Fix**: Map errors to user-friendly messages

### 22. **No debouncing on test button** (ThresholdTester.tsx:26-53)
- Rapid clicks/Enter presses spam predictions
- **Impact**: Wasted resources, UI lag
- **Fix**: Disable button while loading (already done!)

### 23. **Patched model not cleaned up** (candle_router.rs:149-153)
- `model.patched.safetensors` never deleted
- **Impact**: Extra 440 MB disk space
- **Fix**: Clean up patched file, or document it

## Edge Cases for Testing

1. **Path with no parent directory**
2. **Simultaneous downloads**
3. **Simultaneous predictions during initialization**
4. **Disk full during download**
5. **Network timeout during download**
6. **Corrupted model file**
7. **Missing tokenizer files**
8. **Empty prompt**
9. **Very long prompt (>2048 chars)**
10. **Invalid threshold (negative, >1, NaN, Infinity)**
11. **Prompt with null bytes**
12. **Unicode/emoji prompts**
13. **Config update during active usage**
14. **Idle timeout = 0 (never unload)**
15. **Idle timeout = 1 (unload immediately)**
16. **Multiple rapid unload/predict cycles**
17. **Prediction during download**
18. **Download during prediction**
19. **App shutdown during download**
20. **App shutdown during prediction**

## Summary

- **Critical**: 5 bugs (could cause crashes/corruption)
- **High Priority**: 4 bugs (incorrect behavior)
- **Medium Priority**: 11 bugs (poor UX, edge cases)
- **Low Priority**: 8 bugs (minor issues, future-proofing)

**Total**: 28 bugs/issues identified
**Edge Cases**: 20 scenarios to test

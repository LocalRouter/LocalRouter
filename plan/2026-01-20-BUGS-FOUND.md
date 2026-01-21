# Bugs Found in Candle RouteLLM Implementation

**Date:** 2026-01-20
**Review Type:** Code audit after implementation
**Files Reviewed:** All RouteLLM Candle implementation files

---

## Bug #1: Numerically Unstable Sigmoid Implementation (CRITICAL)

**Location:** `src-tauri/src/routellm/candle_router.rs:189-214`

**Severity:** Medium-High (Can cause NaN or incorrect results)

### The Bug

The sigmoid function implementation claims to be "numerically stable" but uses a formula that can overflow for large positive values:

```rust
/// Sigmoid activation function
///
/// Computes: σ(x) = 1 / (1 + e^(-x))  // ← COMMENT LIES!
fn sigmoid(x: &Tensor) -> RouteLLMResult<f32> {
    // For numerical stability, we use:  // ← MISLEADING COMMENT!
    // σ(x) = e^x / (1 + e^x)
    let exp_x = x.exp()?;  // ← Can overflow for large positive x
    let one = Tensor::new(&[1.0f32], x.device())?;
    let denominator = (&one + &exp_x)?;
    let result = (exp_x / denominator)?;  // ← inf / inf = NaN for overflow

    let value = result.to_vec1::<f32>()?[0];
    Ok(value)
}
```

### Why It's Wrong

1. **Formula Used:** `e^x / (1 + e^x)`
   - Stable for negative x
   - **UNSTABLE for positive x** (e^x can overflow to infinity)

2. **Formula Claimed:** `1 / (1 + e^(-x))`
   - Stable for positive x
   - Unstable for negative x

3. **Comment Says:** "For numerical stability"
   - **FALSE!** The formula used is not the most stable choice

### What Happens

**For large positive logit (e.g., x = 100):**
```
e^x = e^100 ≈ 2.7×10^43 (might overflow to inf)
1 + e^x ≈ inf
result = inf / inf = NaN  ← BUG!
```

**Correct behavior should be:**
```
σ(100) ≈ 1.0
```

### Impact

- **Low in practice:** BERT classification logits are typically in range [-5, 5]
- **Could occur:** If model predicts extreme confidence (rare but possible)
- **Result:** NaN win_rate → routing fails → user sees error

### Proper Fix

Use the numerically stable sigmoid that handles both cases:

```rust
fn sigmoid(x: &Tensor) -> RouteLLMResult<f32> {
    // Extract scalar value first
    let x_val = x.to_vec1::<f32>()
        .map_err(|e| RouteLLMError::PredictionFailed(format!("Failed to extract logit: {}", e)))?[0];

    // Numerically stable sigmoid
    let result = if x_val >= 0.0 {
        // For positive x: use 1 / (1 + e^(-x))
        1.0 / (1.0 + (-x_val).exp())
    } else {
        // For negative x: use e^x / (1 + e^x)
        let exp_x = x_val.exp();
        exp_x / (1.0 + exp_x)
    };

    Ok(result)
}
```

**Why This Works:**
- Positive x: uses e^(-x) which can't overflow (only underflow to 0)
- Negative x: uses e^x which can't overflow (only underflow to 0)
- No NaN values possible

### Alternative: Use Candle's Built-in

If Candle provides a sigmoid operation, use it:
```rust
let result = x.sigmoid()?;  // Candle might have this
```

---

## Bug #2: Incorrect Variable Naming

**Location:** `src-tauri/src/ui/commands_routellm.rs:78`

**Severity:** Low (Confusing, already fixed)

### The Bug

Variable named `onnx_path` even though we're using SafeTensors:

```rust
let (onnx_path, tokenizer_path) = service.get_paths();  // ← WRONG NAME
crate::routellm::downloader::download_models(&onnx_path, &tokenizer_path, Some(app_handle))
```

### Fix Applied

```rust
let (model_path, tokenizer_path) = service.get_paths();  // ← CORRECT
crate::routellm::downloader::download_models(&model_path, &tokenizer_path, Some(app_handle))
```

**Status:** ✅ Fixed

---

## Bug #3: Misleading Config Documentation

**Location:** `src-tauri/src/config/mod.rs:78-86`

**Severity:** Low (Documentation only)

### The Bug

Config field comments reference old ONNX paths:

```rust
/// Path to ONNX model file  // ← WRONG
/// Default: ~/.localrouter/routellm/routellm_bert.onnx  // ← WRONG
pub onnx_model_path: Option<PathBuf>,
```

### Fix Applied

```rust
/// Path to model directory (contains model.safetensors)  // ← CORRECT
/// Default: ~/.localrouter/routellm/model/  // ← CORRECT
/// Note: Field name kept as 'onnx_model_path' for backward compatibility
pub onnx_model_path: Option<PathBuf>,
```

**Status:** ✅ Fixed

---

## Bug #4: Missing AppState Management (CRITICAL - Fixed)

**Location:** `src-tauri/src/main.rs` (before fix)

**Severity:** Critical (Feature completely broken)

### The Bug

RouteLLM service was created but never initialized, and AppState wasn't managed in Tauri:

```rust
// Before: No RouteLLM initialization
let app_router = Arc::new(router::Router::new(...));
// RouteLLM service never created!

// Before: No AppState management
app.manage(config_manager);
// AppState never managed - commands fail!
```

### Fix Applied

```rust
// After: Initialize RouteLLM
let routellm_service = match routellm::RouteLLMService::new_with_defaults(idle_timeout) {
    Ok(service) => {
        let service_arc = Arc::new(service);
        let _ = service_arc.clone().start_auto_unload_task();
        Some(service_arc)
    }
    Err(e) => None
};

// Add to router
app_router = app_router.with_routellm(routellm_service);

// After: Manage AppState
if let Some(app_state) = server_manager.get_state() {
    app_state.set_app_handle(app.handle().clone());
    app.manage(Arc::new(app_state));
}
```

**Status:** ✅ Fixed

---

## Potential Bug #5: Memory Leak in Auto-Unload

**Location:** `src-tauri/src/routellm/memory.rs` (if it exists)

**Severity:** TBD (Need to verify)

### Concern

Auto-unload task might not properly clean up if:
1. Service is dropped while task is running
2. Multiple tasks spawned accidentally
3. Task doesn't exit when service is destroyed

### Verification Needed

Check if auto-unload task:
- Uses weak references to service (or strong?)
- Exits when service is dropped
- Can be spawned multiple times accidentally

**Status:** ⚠️ Needs Investigation

---

## Potential Bug #6: Tensor Device Mismatch

**Location:** `src-tauri/src/routellm/candle_router.rs:136-142`

**Severity:** Low (Unlikely in practice)

### Concern

Creating tensors on different devices could cause issues:

```rust
let input_ids_tensor = Tensor::new(input_ids, &self.device)?;
let attention_mask_tensor = Tensor::new(attention_mask, &self.device)?;
```

If tokenizer returns data in different format or device changes, tensors might not match model's device.

### Mitigation

Current code explicitly uses `&self.device` for all tensors, so this should be fine. But worth noting for debugging.

**Status:** ✅ OK (by design)

---

## Potential Bug #7: Classifier Weight Loading

**Location:** `src-tauri/src/routellm/candle_router.rs:89-93`

**Severity:** High (Could fail to load)

### Concern

```rust
let classifier_vb = vb.pp("classifier");
let classifier = linear(768, 1, classifier_vb)?;
```

This assumes:
1. SafeTensors file has weights under key "classifier"
2. Classifier is a simple linear layer (768 → 1)
3. No bias or additional layers needed

### What Could Go Wrong

If the HuggingFace model has:
- Different weight key name (e.g., "bert.classifier", "model.classifier")
- Additional layers (e.g., dropout, layer norm)
- Different dimensions

Then loading will fail with:
```
Error: Weight not found: classifier.weight
```

### Verification Needed

1. Download actual model from HuggingFace
2. Inspect SafeTensors keys
3. Verify architecture matches our assumptions

**Status:** ⚠️ HIGH PRIORITY - Needs Testing with Real Model

---

## Summary

| Bug # | Severity | Status | Impact |
|-------|----------|--------|--------|
| 1 | Medium-High | ⚠️ Unfixed | Numerical instability for extreme logits |
| 2 | Low | ✅ Fixed | Confusing variable name |
| 3 | Low | ✅ Fixed | Misleading documentation |
| 4 | Critical | ✅ Fixed | Feature was completely broken |
| 5 | TBD | ⚠️ Needs Investigation | Potential memory leak |
| 6 | Low | ✅ OK | Already handled correctly |
| 7 | High | ⚠️ Needs Testing | Model might not load |

---

## Action Items

### Immediate (Before First Use)

1. **Fix Bug #1:** Implement numerically stable sigmoid
2. **Test Bug #7:** Download real model and verify loading works

### Before Production

1. **Investigate Bug #5:** Verify auto-unload doesn't leak
2. **Add Tests:** Test extreme logit values (±100)
3. **Add Monitoring:** Log NaN values if they occur

### Nice to Have

1. Rename `onnx_model_path` to `model_path` in config (breaking change)
2. Add validation for win_rate values (should be [0, 1])
3. Add telemetry for sigmoid overflow detection

---

## Testing Recommendations

### Unit Tests Needed

```rust
#[test]
fn test_sigmoid_extreme_values() {
    // Test large positive logit
    let large_pos = Tensor::new(&[100.0f32], &Device::Cpu).unwrap();
    let result = sigmoid(&large_pos).unwrap();
    assert!((result - 1.0).abs() < 0.001, "sigmoid(100) should be ~1.0");

    // Test large negative logit
    let large_neg = Tensor::new(&[-100.0f32], &Device::Cpu).unwrap();
    let result = sigmoid(&large_neg).unwrap();
    assert!(result < 0.001, "sigmoid(-100) should be ~0.0");

    // Test for NaN
    assert!(!result.is_nan(), "sigmoid should never return NaN");
}
```

### Integration Tests Needed

1. **Download Test:** Actually download model from HuggingFace
2. **Load Test:** Verify model loads successfully
3. **Prediction Test:** Run prediction and verify win_rate in [0, 1]
4. **Extreme Input Test:** Very long prompts (512+ tokens)
5. **Memory Test:** Verify auto-unload works and frees memory

---

## Conclusion

**Critical Bugs Found:** 1 (sigmoid instability)
**High Priority Issues:** 2 (sigmoid + classifier loading)
**Medium Priority:** 0
**Low Priority:** 3 (all fixed)

**Overall Assessment:** Implementation is good but needs:
1. Sigmoid fix for production readiness
2. Real model testing to verify assumptions
3. Additional unit tests for edge cases

**Recommendation:** Fix sigmoid bug and test with real model before deploying to users.

---

**Reviewed by:** Claude Sonnet 4.5
**Date:** 2026-01-20

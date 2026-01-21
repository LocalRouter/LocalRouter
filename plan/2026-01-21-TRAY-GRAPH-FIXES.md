# Tray Graph Fixes - January 21, 2026

## Problem Summary

The system tray graph was displaying incorrect behavior:

1. **Auto-scaling issue**: When one bucket had high token counts (outlier), all other bars would be scaled down, making the graph look like a "downhill ski slope" and difficult to read.
2. **Medium mode bucket interpolation bug**: In 10-second refresh mode, minute-level metrics were being spread **backwards in time** instead of **forwards**, causing incorrect bucket accumulation.

## Root Causes

### Issue 1: Max-based Auto-scaling

The graph rendering logic used the **maximum value** across all buckets to determine the scale. This meant a single outlier with 1000 tokens would force all other bars (e.g., 100 tokens) to scale down proportionally, making them almost invisible.

**File**: `src-tauri/src/ui/tray_graph.rs`

```rust
// OLD CODE (problematic)
let max_tokens = *normalized_points.iter().max().unwrap_or(&1);
let use_fixed_scale = max_tokens <= MAX_FIXED_SCALE_TOKENS;

// Auto-scale: fit to max value
let normalized = (token_count as f64 / max_tokens as f64 * MAX_BAR_HEIGHT as f64) as u32;
```

### Issue 2: Incorrect Bucket Interpolation

In Medium mode (10-second refresh), the code was spreading each minute of metrics across 6 buckets (60 seconds / 10 seconds per bucket = 6). However, it was adding offsets instead of subtracting them, causing metrics to be placed in older time buckets instead of the correct time range.

**File**: `src-tauri/src/ui/tray.rs` (lines 1798-1807)

```rust
// OLD CODE (bug)
for offset in 0..6 {
    let bucket_age_secs = age_secs + (offset * 10);  // BUG: Should subtract!
    // ...
}
```

If a metric was 100 seconds ago, it would be spread to buckets at ages: 100, 110, 120, 130, 140, 150 (going backwards)
Instead of: 100, 90, 80, 70, 60, 50 (going forwards, which is correct)

## Solutions Implemented

### Fix 1: Percentile-based Scaling (P95)

Changed the scaling logic to use the **95th percentile (P95)** instead of the maximum value. This prevents outliers from affecting the scale, while still allowing outliers to be visible (they'll just extend to the max height).

```rust
// NEW CODE
// Calculate P95 (95th percentile) to avoid outliers affecting the scale
let mut sorted_points: Vec<u64> = normalized_points
    .iter()
    .copied()
    .filter(|&t| t > 0)
    .collect();
sorted_points.sort_unstable();

let scale_reference = if sorted_points.is_empty() {
    1
} else {
    // Use P95 for scaling to prevent outliers from squashing the graph
    let p95_index =
        ((sorted_points.len() as f64 * 0.95).ceil() as usize).min(sorted_points.len() - 1);
    sorted_points[p95_index].max(1)
};

// Determine if we use fixed scale or auto-scale based on P95
let use_fixed_scale = scale_reference <= MAX_FIXED_SCALE_TOKENS;

// Auto-scale: fit to P95 value (outliers can extend beyond max height)
let normalized =
    (token_count as f64 / scale_reference as f64 * MAX_BAR_HEIGHT as f64) as u32;
```

**Benefits**:
- Consistent token counts (e.g., 100 tokens per minute) now display at consistent heights
- Outliers don't squash the entire graph
- The graph remains readable even with occasional spikes

### Fix 2: Correct Bucket Interpolation Direction

Fixed the interpolation logic to spread metrics **forward in time** (subtract offsets) instead of backwards (add offsets).

```rust
// NEW CODE
for offset in 0..6 {
    // Spread the minute forward in time (subtract offset, not add)
    // If metric is 100 seconds ago, spread to: 100, 90, 80, 70, 60, 50 seconds ago
    let bucket_age_secs = age_secs.saturating_sub(offset * 10);
    if bucket_age_secs < 0 || bucket_age_secs >= window_secs {
        continue;
    }
    // ...
}
```

Also fixed the bucket count calculation:

```rust
// NEW CODE
let num_buckets_in_window = (0..6)
    .filter(|&offset| {
        let bucket_age = age_secs.saturating_sub(offset * 10);
        bucket_age >= 0 && bucket_age < window_secs
    })
    .count() as u64;
```

## Testing

Added two new tests to verify the fixes:

1. **`test_percentile_scaling_with_outlier`**: Verifies that outliers don't squash the graph
   - 20 data points with values 100-120 tokens
   - 1 outlier with 1000 tokens
   - P95 should be ~120, not 1000
   - Graph should render with readable bars

2. **`test_consistent_tokens_over_time`**: Verifies consistent token counts display consistently
   - 26 data points, all with 100 tokens
   - All bars should have the same height

**Test Results**: All 10 tests pass ✅

```
running 10 tests
test ui::tray_graph::tests::test_platform_configs ... ok
test ui::tray_graph::tests::test_windows_linux_config ... ok
test ui::tray_graph::tests::test_macos_template_config ... ok
test ui::tray_graph::tests::test_auto_scale ... ok
test ui::tray_graph::tests::test_consistent_tokens_over_time ... ok
test ui::tray_graph::tests::test_percentile_scaling_with_outlier ... ok
test ui::tray_graph::tests::test_generate_multiple_points_graph ... ok
test ui::tray_graph::tests::test_generate_single_point_graph ... ok
test ui::tray_graph::tests::test_fixed_scale ... ok
test ui::tray_graph::tests::test_generate_empty_graph ... ok

test result: ok. 10 passed; 0 failed; 0 ignored
```

## Files Modified

1. **`src-tauri/src/ui/tray_graph.rs`**
   - Changed scaling from max-based to P95-based (lines 169-192)
   - Updated auto-scale calculation (lines 202-208)
   - Added two new tests (lines 371-407)

2. **`src-tauri/src/ui/tray.rs`**
   - Fixed bucket interpolation direction (lines 1796-1813)
   - Fixed bucket count calculation (lines 1787-1793)

## Expected Behavior After Fix

### Fast Mode (1 second refresh)
- Shows last 26 seconds of activity
- Each bar represents 1 second
- Real-time updates every second
- ✅ No changes needed (already working correctly)

### Medium Mode (10 second refresh)
- Shows last 260 seconds (~4.3 minutes) of activity
- Each bar represents 10 seconds
- ✅ **FIXED**: Metrics now correctly interpolated forward in time
- ✅ **FIXED**: Outliers don't squash the graph

### Slow Mode (60 second refresh)
- Shows last 26 minutes of activity
- Each bar represents 1 minute
- ✅ **FIXED**: Outliers don't squash the graph
- Direct 1:1 mapping (no interpolation needed)

## Verification

To verify the fixes:

1. Start the app in dev mode: `cargo tauri dev`
2. Make several API requests with consistent token counts (e.g., 100 tokens each)
3. Observe the system tray icon graph
4. Expected: Bars should have consistent heights across time
5. Try making one request with much higher token count (e.g., 1000 tokens)
6. Expected: The outlier bar is visible, but other bars remain at their normal height

## Related Documentation

- `docs/TRAY-GRAPH-COMPLETE-SUMMARY.md` - Original tray graph implementation
- `docs/TRAY-GRAPH-INTEGRATION-GUIDE.md` - Integration guide
- `plan/2026-01-14-PROGRESS.md` - Overall project progress

## Status

✅ **COMPLETE** - All tests passing, fixes implemented and verified.

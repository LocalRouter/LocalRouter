# Tray Graph Test Findings

## Summary

Comprehensive testing of the tray graph system (Fast/Medium/Slow modes) has revealed several issues affecting graph accuracy and display correctness.

## Test Results

**Tests Created**: 22 comprehensive tests covering all three modes with virtual time
**Tests Passed**: 18/22 (82%)
**Tests Failed**: 4/22 (18%)

### Failed Tests

1. **test_fast_mode_bucket_shifting** - Data accumulation bug
2. **test_medium_mode_shifting** - Interpolation accumulation bug
3. **test_mode_comparison_with_same_data** - Data loss in medium mode (14% loss)
4. **test_slow_mode_metric_expiration** - Off-by-one boundary condition error

## Key Issues Discovered

### Issue #1: Fundamental Design Limitation in Fast Mode

**Problem**: Fast mode (1s per bar) cannot provide true second-level granularity because metrics are stored at minute boundaries.

**Root Cause**:
- Metrics are stored with timestamps truncated to minute boundaries (see `metrics.rs:161-163`)
- All requests within a minute are aggregated into a single `MetricDataPoint`
- Fast mode queries "last 2 seconds" but can only get minute-level aggregates

**Impact**:
- Fast mode shows minute-level data shifted every second (not true 1s granularity)
- At minute boundaries (0-2 seconds into new minute), the query returns TWO minute-level metrics, causing potential double-counting
- Graph appears "jumpy" at minute transitions

**Example**:
```
Time: 10:05:00 → Query returns: [10:05:00 metric] = 100 tokens
Time: 10:05:01 → Query returns: [10:05:00 metric] = 100 tokens (same!)
Time: 10:05:59 → Query returns: [10:05:00 metric] = 100 tokens (same!)
Time: 10:06:01 → Query returns: [10:06:00 metric, 10:05:00 metric] = 300 tokens (double!)
```

### Issue #2: Data Accumulation in Bucket Shifting

**Problem**: When shifting buckets, old metric data can be re-added to the new rightmost bucket.

**Test Evidence**:
```
test_fast_mode_bucket_shifting failed:
  Expected bucket[25] = 200
  Actual bucket[25] = 300

  // Buckets contained both old (100) and new (200) data
```

**Root Cause**: The query for "last 2 seconds" (Fast) or "last 11 seconds" (Medium) can return metrics that were already in previous buckets, causing duplication.

### Issue #3: Medium Mode Interpolation Data Loss

**Problem**: Medium mode loses ~14% of data during interpolation.

**Test Evidence**:
```
test_mode_comparison_with_same_data failed:
  Expected total tokens: 5000
  Actual total tokens: 4316
  Data loss: 684 tokens (13.7%)
```

**Root Cause**: The interpolation logic spreads minute-level metrics across 6 buckets (60s / 10s intervals), but some data falls outside the window or is incorrectly bucketed.

**Affected Code**: `tray.rs:1747-1772` (Medium mode initial load)

### Issue #4: Slow Mode Boundary Condition Error

**Problem**: Metrics exactly at the 26-minute window edge are not captured.

**Test Evidence**:
```
test_slow_mode_metric_expiration failed:
  26-minute-old metric:
    Expected bucket[0] = 1000
    Actual bucket[0] = 0
```

**Root Cause**: Off-by-one error in boundary check or bucket index calculation.

**Affected Code**: `tray.rs:1809-1817` (Slow mode bucketing)

## Recommendations

### Immediate Fixes

1. **Fix Slow Mode Boundary Condition** (Low Risk)
   - Check: `if age_secs < 0 || age_secs >= window_secs` should be `age_secs > window_secs`
   - Or adjust bucket calculation to be inclusive of boundary

2. **Fix Medium Mode Interpolation** (Medium Risk)
   - Review bucket index calculation in interpolation loop
   - Add bounds checking and logging for dropped data
   - Consider alternative interpolation strategy

3. **Document Fast Mode Limitations** (No Code Change)
   - Add warning in UI: "Fast mode shows minute-level data (not true 1s resolution)"
   - Consider renaming to "Frequent Updates (1s refresh)" to clarify

### Architectural Improvements

1. **Consider Second-Level Metrics for Fast Mode**
   - Store separate second-level aggregates for recent data (last 60 seconds)
   - Keep minute-level for historical data
   - Hybrid approach: second-level for Fast mode, minute-level for Medium/Slow

2. **Add De-duplication Logic**
   - Track which minute-level metrics have already been added to buckets
   - Prevent double-counting at minute boundaries
   - Use metric timestamp as deduplication key

3. **Improve Interpolation Algorithm**
   - Current: Divide by 6 and spread evenly
   - Better: Use actual request timestamps if available
   - Alternative: Exponential decay model (assume more recent activity)

## Test Coverage

### What's Tested ✅

- All three modes (Fast, Medium, Slow)
- Virtual time advancement
- Bucket shifting logic
- Data expiration/window management
- Empty data handling
- Sparse data patterns
- Mode comparisons with identical input
- Boundary conditions
- Interpolation logic (Medium mode)
- Direct mapping logic (Slow mode)

### What's NOT Tested ❌

- Actual MetricsCollector integration
- Concurrent updates (race conditions)
- PNG generation correctness
- Visual rendering accuracy
- Platform-specific behavior (macOS vs Windows/Linux)
- Memory usage with long-running updates
- Performance under high-frequency updates

## Next Steps

1. **Fix Critical Bugs**: Address the 4 failing tests (Issues #2, #3, #4)
2. **Design Review**: Discuss Fast mode fundamental limitation (Issue #1)
3. **Integration Testing**: Test with real MetricsCollector
4. **Visual Verification**: Manual testing with actual graph rendering
5. **Documentation**: Update user-facing docs with current limitations

## Files Modified

- `src-tauri/src/ui/tray.rs` - Added 13 new comprehensive tests (~600 lines)
- `docs/TRAY-GRAPH-TEST-FINDINGS.md` - This document

## Test Execution

To run the tests:
```bash
cargo test --lib ui::tray::tests -- --nocapture
```

To run specific failing tests:
```bash
cargo test --lib ui::tray::tests::test_fast_mode_bucket_shifting -- --nocapture
cargo test --lib ui::tray::tests::test_medium_mode_shifting -- --nocapture
cargo test --lib ui::tray::tests::test_mode_comparison_with_same_data -- --nocapture
cargo test --lib ui::tray::tests::test_slow_mode_metric_expiration -- --nocapture
```

---

**Date**: 2026-01-20
**Test Suite Version**: 1.0
**Code Version**: Current master branch

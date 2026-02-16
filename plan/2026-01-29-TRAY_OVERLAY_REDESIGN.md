# Plan: Tray Icon Overlay Redesign

## Summary
Move the exclamation mark overlay to top-left corner, add a down-arrow for update-available, and carve out the top-left corner of the rounded rect box with an inverted (concave) radius when an overlay is present.

## Files to Modify

### 1. `src-tauri/src/ui/tray_graph.rs`

**Add overlay enum:**
```rust
pub enum TrayOverlay {
    None,
    Warning(Rgba<u8>),   // exclamation mark with status color
    UpdateAvailable,      // down arrow in foreground color
}
```

**Change `generate_graph` signature:**
- Replace `health_status: Option<AggregateHealthStatus>` with `overlay: TrayOverlay`
- Caller constructs the overlay enum

**Modify top-left corner of rounded rect:**
- When overlay is `None`: draw normal top-left corner (current 6px radius)
- When overlay is `Warning` or `UpdateAvailable`: replace top-left corner with a larger concave (inverted) cutout — approximately 10-11px radius carved inward, creating space for the indicator icon
- The concave cutout: instead of the border curving outward, it curves inward. Pixels that would normally be inside the box near the top-left are cleared (set to transparent/background), and the border follows a concave arc

**Rewrite `draw_exclamation_mark`:**
- Position in top-left corner area (~1,1 to ~9,9) instead of centered
- Scale down to fit the carved-out area
- Keep the same stem + dot design but smaller

**Add `draw_down_arrow`:**
- Position in top-left corner area (~1,1 to ~9,9)
- Simple downward-pointing arrow/chevron shape
- Uses foreground color

**Clip graph bars:** Ensure bars don't render into the carved-out top-left area (the area is near y=0 which is the top of the image — bars grow from the bottom, so this should naturally not overlap unless bars are very tall; add a check just in case).

### 2. `src-tauri/src/ui/tray.rs`

**In the graph update loop (~line 1566-1574):**
- Also fetch `UpdateNotificationState` to check `is_update_available()`
- Determine overlay priority: Warning/Error > UpdateAvailable > None
- Construct `TrayOverlay` enum and pass to `generate_graph`

## Priority Logic
- If health status is Yellow or Red → `TrayOverlay::Warning(color)`
- Else if update available → `TrayOverlay::UpdateAvailable`
- Else → `TrayOverlay::None`

## Visual Design (32x32 pixel grid)

**Normal (no overlay):** Current rounded rect with 6px corner radius at all corners.

**With overlay:** Top-left corner has a concave arc (~10px radius) curving inward, creating a notch. The overlay icon (exclamation or arrow) sits in this notch area, visually "outside" the box but adjacent to it.

## Verification
1. `cargo test -p localrouter-tauri` — ensure existing tray_graph tests pass
2. Run the ignored `write_test_graph_to_file` test to visually inspect: `cargo test -p localrouter-tauri write_test_graph -- --ignored`
3. `cargo clippy -p localrouter-tauri` — no warnings
4. Visual check: run `cargo tauri dev` and observe tray icon with/without health issues

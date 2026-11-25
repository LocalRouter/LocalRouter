# Tray icon color mode

## Goal

Offer a choice of how the tray graph panels are colored:

- **Monochrome** (default, unchanged): one tint. On macOS the icon is a
  template image, so it follows the menu bar's light/dark appearance.
- **Color**: vendor colors. Anthropic is Claude orange (`#D97757`), OpenAI is
  blue (`#3B82F6`).
  - Usage-window gauges (and their numbers) fill with their account's
    vendor color (`anthropic:*` → orange, `openai:*` → blue).
  - The `All` request graph is a stacked graph: Anthropic traffic at the
    bottom, OpenAI above it, everything else on top in the foreground tint.
  - Provider and model panes take their provider's vendor color; client
    panes stay in the foreground (no per-client vendor breakdown exists in
    the metrics store).

## Design

- `lr_config::TrayColorMode { Monochrome, Color }`, stored as
  `ui.tray_stats.color_mode` (serde default → old configs read Monochrome).
- `src-tauri/src/ui/tray_colors.rs`: `TrayVendor` (colors, mapping from
  `ProviderType` — Anthropic; OpenAI and ChatGPT Plus — and from usage
  account ids via `AccountRef::from_id`).
- Renderer (`tray_graph.rs`): `PaneContent::StackedGraph(Vec<GraphLayer>)`
  and `PaneSpec::accent`. Stacked bars are as tall as the plain graph of the
  layer sum and split proportionally (cumulative rounding).
- Manager: in color mode with the `All` graph shown, every Anthropic/OpenAI
  provider instance is tracked as a hidden `TraySource::Provider` alongside
  the displayed sources (same bucket machinery in all refresh modes). Their
  series are summed per vendor; the remainder of `All` is the top layer.
- macOS: the color-mode icon is installed as a non-template image. Its frame
  and labels are drawn white or black from the status item button's
  `effectiveAppearance` (probed on the main thread each tick and on
  `tauri://theme-changed`; a change forces a redraw).
- Frames and gauge outlines/ticks are toned down in color mode
  (`GraphConfig::frame`: the foreground at alpha 110, reading as gray on
  light and dark menu bars) so the colored data stands out; labels and fills
  stay at full strength.
- Settings → Appearance: "Colors" radio (Monochrome / Color); the preview
  renders the stacked split with dummy data.

## Steps

- [x] Config enum + tests
- [x] Renderer: stacked graph + accent + tests
- [x] Vendor mapping module + tests
- [x] Manager wiring, macOS non-template + appearance probe
- [x] TS types, settings UI, demo mock
- [x] Plan review, test coverage review, bug hunt
- [x] Local validation: clippy (workspace, all targets), fmt, `localrouter`
      lib tests, `lr-config` tests, `tsc`, `npm run test:unit`,
      `npm run build`, `npm --prefix website run build:check`
- [x] Commit and push

# Embedded provider logos

## Todo
- [x] Inspect embedded provider icon mappings and official upstream branding.
- [x] Bundle available official logos; remove arbitrary emoji branding where no official logo exists.
- [x] Record upstream asset sources and verify app/demo use local assets.
- [x] Plan Review: check implementation against scope and handle missed cases.
- [x] Test Coverage Review: verify mappings/assets and identify any meaningful uncovered behavior.
- [x] Bug Hunt: inspect matching, rendering, image sizing, and offline behavior.
- [x] Validate frontend builds and exact stable Rust clippy/fmt/workspace tests.
- [x] Commit and push task changes and any automatic catalog update, preserving unrelated changes.

Use original upstream brand assets, not generated replacements. The shared public/icons directory supplies both app and website demo. Check Ollaya, Laya, Kev, Von, Decider, stable-diffusion.cpp, and llama.cpp embedded mappings. Where no project logo is published, use the existing generic provider fallback rather than invent branding.

## Implementation and review
- Bundled original Ollaya owl SVG, Laya spiral mark SVG, and stable-diffusion.cpp PNG, with pinned source URLs and original upstream licenses.
- Added explicit embedded llama.cpp mapping to the existing llama.cpp asset.
- Removed embedded provider emoji fallbacks. Kev, Von, and Decider have no published project logo in the inspected upstream repositories/READMEs and use the existing category icon. Kev's playground favicon is a Vercel starter logo.
- Preserved image aspect ratios with object-contain, including the rectangular stable-diffusion.cpp logo.
- Verified rendered markup for every embedded provider, asset presence in both production builds, and existing fallback behavior. This static asset/mapping change needs no new persistent test suite.
- Rust validation uses the stable toolchain bin directory first in PATH and clears the broken system RUSTC_WRAPPER, ensuring Homebrew cargo-clippy is not selected.
- Validation passed: app and website production builds, npx tsc --noEmit, rendered icon mapping/fallback checks, bundled assets in both outputs, light/dark visual inspection, stable workspace Clippy with warnings denied, stable formatting check, and stable workspace tests including doctests.
- The build automatically refreshed modelsdev_raw.json to 8,383 models; include this generated change as required by AGENTS.md.

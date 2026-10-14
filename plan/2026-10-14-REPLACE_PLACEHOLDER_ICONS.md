# Progress / todo

- [x] Inspect shared icons, provider types, demo usage, and current repository state.
- [x] Save this plan with copy-plan.sh before implementation.
- [x] Audit all service/provider/client/MCP icon placeholders and identify missing official assets.
- [x] Obtain official logos from upstream repositories or websites and record sources and licenses.
- [x] Remove emoji/symbol placeholders from shared icon rendering and use established UI icons for unbranded services and load failures.
- [x] Verify icon matching, fallback rendering, and changes of service after a load failure.
- [x] Plan review: check every discovered placeholder and app/demo asset paths against implementation.
- [x] Test coverage review: add meaningful coverage for changed matching/fallback behavior and use existing icon-serving tests.
- [x] Bug hunt: review icon matching for accidental unrelated branding, image failures, accessibility, and theme contrast.
- [x] Validate app and website types/builds and required Rust stable update, clippy, fmt, and workspace tests using the pinned compiler path.
- [x] Commit and push validated task changes plus any automatic catalog update, preserving unrelated changes.

# Replace remaining placeholder icons

Audit the shared ServiceIcon map used by providers, client templates, MCP
servers, and the website demo. Add official locally bundled logos where
available, especially TypeSafe and Vercel AI Gateway. Remove the entire emoji
fallback map so known and unknown services, including System One compatible
servers, use the established provider/server icons when no logo is available
or a logo fails to load. Preserve existing icon sizing and dark-theme contrast.

Check generic compatible provider identifiers before substring matching so they
do not accidentally inherit OpenAI branding. Ensure changing the service after
an image error renders the next logo normally. Record provenance and applicable
licenses for every new bundled logo. Audit other UI/demo icon sources for
similar placeholders without rewriting ordinary prose or status text.

## Review and validation

- Removed all 92 shared emoji/symbol fallback entries and replaced 20 emoji
  values in MCP template metadata with service identifiers. Template buttons
  now resolve their explicit icon identifier through the shared renderer.
- Added original TypeSafe AI and Vercel site assets and documented their
  sources and applicable proprietary/brand terms in `public/icons/SOURCES.md`.
  TypeSafe SHA-256: `fcb8f554778c45ff62b97ca05df643588b39c8c66913736b7fe3485cedd96180`.
  Vercel SHA-256: `27622083391fe7729d41233aa75e24fa6f7ea25b37f43f6cb1b7917f0be0a2dc`.
- Generic compatible endpoints have explicit neutral mappings. Matching checks
  the longest service identifier first to avoid unrelated shorter branding.
  Object prototype property names also resolve to the generic icon.
- The logo container is keyed by its asset so a previously hidden broken
  image cannot suppress a different service's logo. Browser checks verify
  failure fallback, changing services after failure, provider/MCP category
  icons, both themes, and the TypeSafe/Vercel provider picker assets.
- All 72 mapped assets exist locally. Both production outputs contain original
  copies of the two new logos. No remote runtime assets were introduced.
- App and website TypeScript/production builds pass. Ten targeted frontend
  tests pass, including the new icon matching regressions and existing shared
  icon-serving checks. Full Rust checks pass on stable 1.99.0 with PATH and
  RUSTC pinned: clippy with warnings denied, fmt, and 4,144 workspace tests.
- Final plan/coverage reviews and bug hunt are complete; diff whitespace checks
  pass. No catalog update was produced. Pre-existing release plans remain
  outside this commit.

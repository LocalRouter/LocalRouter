# Progress / todo

- [x] Inspect provider icon mapping, shared website assets, and current repository state.
- [x] Save this plan with copy-plan.sh before changing implementation files.
- [x] Obtain LLM Gateway's official logo at a pinned upstream revision and record its license.
- [x] Bundle the asset locally and replace the provider's emoji mapping.
- [x] Plan review: confirm the app and demo both resolve the official icon and no remote runtime asset is introduced.
- [x] Test coverage review: use existing type/build checks and inspect rendering in light and dark themes; no new behavioral code is expected.
- [x] Bug hunt: inspect the final mapping, SVG contents, source attribution, and packaged assets.
- [x] Validate app/website TypeScript and builds, then run the required stable Rust update, clippy, fmt, and workspace tests with the compiler path pinned.
- [x] Commit and push the validated task changes and any automatic catalog update to the configured upstream, preserving unrelated files.

# LLM Gateway provider icon

Replace the placeholder in `src/components/ServiceIcon.tsx` with the official
LLM Gateway logo from its own repository. Reuse the existing provider icon
rendering and website shared-icons pipeline. Bundle the original SVG and
license under `public/icons/`, with a pinned source recorded in `SOURCES.md`.

The branch initially trails its upstream by two release metadata commits.
Fast-forward before validation. Two pre-existing untracked release plans are
unrelated and must remain untouched.

## Review and validation

- The official black symbol is byte-identical to upstream revision
  `ba743696ea6db4b7a68ba764ddd70edc19fbb95d`; SHA-256:
  `dae73b4fee412926836b1c3bc4b41ed799f3b105184c3a585e0a2bb6d798a82d`.
- SVG inspection confirms it contains only vector paths, without scripts or
  external references. The provider uses the shared local asset mapping.
- Browser screenshots confirm the provider picker renders correctly in light
  and dark themes, including the existing light panel behind the black symbol.
- Browser checks confirm the configuration dialog renders the same SVG at
  24px and a failed image displays the established provider icon, without an emoji.
- Existing frontend regressions pass (8 tests), including shared icon serving.
  This asset/mapping change adds no behavioral branches needing new unit tests.
- App and website type checks and production builds pass. Both outputs contain
  byte-identical copies of the original SVG and upstream license notice.
- Rust stable is 1.99.0; validation pins both PATH and RUSTC to its actual
  rustup compiler path. `rustup update stable`, full-workspace clippy with
  warnings denied, formatting checks, and workspace tests all pass
  (4,144 tests passed, no failures).
- Final diff review is clean. No catalog update was produced. Unrelated release
  plans and the concurrent gateway roadmap are preserved outside this commit.

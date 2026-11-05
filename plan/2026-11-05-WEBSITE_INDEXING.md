# Website indexing and deployment

- [x] Inspect public routes, rendering, metadata and deployment pipeline.
- [x] Generate static HTML for all public marketing, documentation and research routes from existing React content; retain interactive client rendering.
- [x] Provide unique route titles, descriptions and canonical URLs; noindex demo and missing pages; serve a real 404 fallback.
- [x] Generate a complete sitemap from the same route data, with honest source modification dates or omit dates.
- [x] Update deployment to publish generated route pages without overwriting them.
- [x] Plan Review: compare implementation with this plan and resolve gaps.
- [x] Test Coverage Review: add meaningful regressions for static content, metadata, sitemap coverage, unknown routes and client navigation.
- [x] Bug Hunt: review routing, escaping, build failures and deployment behavior.
- [x] Run website build and relevant tests; verify in a browser.
- [ ] Commit and push: preserve unrelated work, merge with --no-ff into master, and push normally.
- [ ] Monitor the exact website deployment to completion and verify live HTML, assets, sitemap and robots.

Search ranking and Google-owned index state cannot be controlled by a deployment. Search Console ownership and submission require authenticated property access; report any remaining limitation.

## Implementation and review

- Build renders 33 indexable public routes plus a noindex demo shell, docs alias and true-404 template, from existing React content.
- Route metadata and sitemap share the documentation/research registries. Sitemap dates are omitted instead of fabricated.
- Deployment publishes generated HTML directly, and a broken homepage JSON repair link now targets the existing research paper.
- Plan Review, Test Coverage Review and Bug Hunt completed: verified trailing slashes, metadata escaping and uniqueness, sitemap completeness, missing routes, noindex recovery, static assets, internal links, SSR failures and interactive documentation previews.
- Local Node 20 validation: website build/typecheck/static checks pass; Chromium verifies all 33 pages without JavaScript plus client navigation, demo, missing-page handling and interactive docs. Build retains existing chunk-size/dynamic-import warnings.
- No Rust or desktop code changes; Rust and desktop suites are not run for this website-only task.

Deployment verification is recorded after publication.

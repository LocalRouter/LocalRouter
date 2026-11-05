# Static website publishing

`npm run build` builds the Vite client and renders the public React pages to
route-specific HTML. `npm run build:check` also type-checks and verifies the
output. Install both the repository root and website dependencies first.

`src/site-pages.ts` derives documentation and research routes from their existing
section and paper lists. It supplies the same titles, descriptions and canonical
URLs to static generation and client navigation. Every indexable route gets a
physical `index.html` and a sitemap entry. URLs use trailing slashes to match
GitHub Pages directory URLs. Modification dates are omitted rather than guessed.

The demo keeps a client-rendered shell with `noindex`; the generated `404.html`
contains a missing-page view with `noindex` and no canonical. `/docs/` is an alias
that points to `/docs/introduction/`. Interactive documentation previews mount
only in the browser; the complete article text is rendered at build time. The
client uses `createRoot` to mount over static content, so browser-dependent demos
and stored themes do not need to match a server hydration snapshot.

GitHub Pages deploys `dist/` directly. Do not replace generated route indexes or
`404.html` with copies of the homepage. CNAME, robots.txt, sitemap.xml and
.nojekyll are included by the build.

`npm run test:browser` checks every public page with JavaScript disabled and
checks client navigation, metadata, interactive docs, demo and missing-page
behavior. It uses the repository root's existing Playwright dependency; install
its browser with `npx playwright install chromium` from the root if necessary.

A deployment makes content available to crawlers. Google Search Console's URL
Inspection and sitemap report are needed to confirm Google's chosen canonical,
indexing status and discovery. No analytics or tracking is added.

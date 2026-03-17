# Docs Revamp Plan

## Context
The website docs (`/docs`) currently renders all 16 sections on a single scrollable page. The content is dense (most entries are single long paragraphs), the sidebar is flat (no grouping), and subsection titles use a `ChevronRight` icon that looks like a collapsible toggle. This makes the docs hard to navigate and read.

## Changes

### 1. Separate Pages per Section

**Files**: `website/src/App.tsx`, `website/src/pages/Docs.tsx`

- Change route from `/docs` → `/docs/:sectionId` (e.g., `/docs/introduction`, `/docs/getting-started`)
- `/docs` redirects to `/docs/introduction`
- Each page renders only one section's content (its subsections and children)
- Add prev/next navigation links at the bottom of each page
- Use React Router `useParams()` to determine which section to render
- Keep the existing `sections` data structure in `Docs.tsx`

### 2. Sidebar with Separators and Indentations

**Files**: `website/src/pages/Docs.tsx` (Sidebar component)

Group the 16 sections into categories with visual separator labels:

| Group | Sections |
|-------|----------|
| **Getting Started** | Introduction, Getting Started |
| **Core Features** | Clients, Providers, Model Selection & Routing, Rate Limiting |
| **MCP & Extensions** | Unified MCP Gateway, Skills, Marketplace |
| **Security** | Firewall, GuardRails, Privacy & Security |
| **Operations** | Monitoring & Logging, Configuration |
| **API Reference** | API Reference: OpenAI Gateway, API Reference: MCP Gateway |

Implementation:
- Add a `sidebarGroups` array that maps group labels to section IDs
- Render group labels as small uppercase muted text separators
- Section items become `<Link to={/docs/${id}}>` instead of scroll buttons
- When a section is active, show its subsections indented below it in the sidebar
- Subsections link to `#subsection-id` anchors within the page

### 3. Content Readability — Split Paragraphs in Markdown Files

**Files**: All 16 files in `website/src/pages/docs/content/*.md`

Nearly every `@entry` is a single dense paragraph. Split them at natural topic boundaries:
- Separate "what it is" from "how it works" from "why it matters"
- Break after sentences that end a concept and before sentences that start a new one
- Add markdown `**subtitles**` or `### headings` where a paragraph covers multiple distinct topics
- Keep technical accuracy intact

### 4. Replace ChevronRight Icon on Subsection Titles

**Files**: `website/src/pages/Docs.tsx` (SectionContent component)

Replace the `ChevronRight` icon on `<h3>` subsection titles. Options:
- Use a `Hash` icon (`#`) — matches common docs convention
- Or remove the icon entirely and rely on styling alone

The current icon looks like a collapsible/expandable toggle, which is confusing since clicking does nothing.

## Verification
1. Run `cd website && npm run dev` and navigate to `/docs`
2. Verify redirect from `/docs` to `/docs/introduction`
3. Click through sidebar sections — each should load as its own page
4. Verify sidebar groups have separator labels and correct indentation
5. Verify subsections appear indented in sidebar when section is active
6. Check that prev/next navigation works at the bottom of each page
7. Verify content is more readable with split paragraphs
8. Verify no `ChevronRight` icons on subsection titles
9. Check mobile sidebar still works (overlay mode)
10. Verify deep links work (e.g., `/docs/providers#circuit-breaker`)

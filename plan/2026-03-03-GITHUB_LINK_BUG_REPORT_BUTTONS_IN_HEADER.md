# Plan: GitHub Link + Bug Report Buttons in Header

## Context

The app header currently has Search, Theme toggle, and Help buttons. The user wants to add a GitHub link button and a Bug Report button with a native dialog that captures a screenshot, optionally includes sanitized config, and opens a prepopulated GitHub issue.

---

## Step 1: Install `html-to-image`

```bash
npm install html-to-image
```

Lightweight DOM-to-image library (~8KB). CSP already allows `data:` and `blob:` in `img-src`.

---

## Step 2: Create `src/components/layout/BugReportDialog.tsx`

New component that renders both the trigger button and a controlled Dialog.

**Flow:**
1. User clicks Bug icon button
2. `toPng(document.documentElement)` captures the DOM **before** dialog opens
3. Dialog opens with screenshot preview + form
4. On submit: copies screenshot to clipboard (if included), builds GitHub issue URL, opens in browser

**Key elements:**
- Screenshot preview with `<img>` tag, dimmed (opacity-40) when checkbox unchecked
- `Checkbox` + `Label` for "Include screenshot" (unchecked by default)
- `Checkbox` + `Label` for "Include configuration" with note "(API keys and secrets are removed)" (unchecked by default)
- `Input` for title
- `Textarea` for description
- `Button` "Open GitHub Issue" with `ExternalLink` icon (clearly indicates it goes to GitHub)
- Cancel button

**Config sanitization** — recursive function that walks the JSON and replaces values for keys matching `/api_key|secret|token|password|^key$|auth_header|credential|private_key/i` with `"[REDACTED]"`.

**GitHub URL** — `https://github.com/LocalRouter/LocalRouter/issues/new?title=...&body=...`
- Body includes: description, app version (`get_app_version`), OS (navigator.userAgent), screenshot paste note, sanitized config in `<details>` block
- URL length check: if >8000 chars, truncate/strip config section

**Screenshot clipboard** — convert data URL to blob, use `navigator.clipboard.write([new ClipboardItem({"image/png": blob})])`, show toast "Screenshot copied to clipboard — paste it into the GitHub issue"

**Imports to reuse:**
- `Dialog*` from `@/components/ui/dialog`
- `Checkbox` from `@/components/ui/checkbox`
- `Label` from `@/components/ui/label`
- `Input` from `@/components/ui/Input`
- `Textarea` from `@/components/ui/textarea`
- `Button` from `@/components/ui/Button`
- `cn` from `@/lib/utils`
- `invoke` from `@tauri-apps/api/core`
- `open` from `@tauri-apps/plugin-shell`
- `toPng` from `html-to-image`
- `Bug`, `ExternalLink` from `lucide-react`
- `toast` from `sonner`

---

## Step 3: Modify `src/components/layout/header.tsx`

Add between Theme toggle and Help button:

```tsx
import { Github } from "lucide-react"
import { BugReportDialog } from "@/components/layout/BugReportDialog"

{/* Bug Report */}
<BugReportDialog />

{/* GitHub */}
<Button variant="ghost" size="icon" onClick={() => open("https://github.com/LocalRouter/LocalRouter")}>
  <Github className="h-4 w-4" />
  <span className="sr-only">GitHub repository</span>
</Button>
```

Final button order (left to right): **Search, Theme, Bug, GitHub, Help**

---

## Step 4: Demo mock — `website/src/components/demo/TauriMockSetup.ts`

Add `get_config` mock handler returning a mock config object with a fake API key (so sanitization can be demonstrated).

---

## Key Files

| File | Change |
|------|--------|
| `src/components/layout/BugReportDialog.tsx` | **New** — Bug report dialog component |
| `src/components/layout/header.tsx` | Add Bug + GitHub buttons |
| `website/src/components/demo/TauriMockSetup.ts` | Add `get_config` mock |
| `package.json` | Add `html-to-image` dependency |

**Existing components reused (no changes):**
- `src/components/ui/dialog.tsx`
- `src/components/ui/checkbox.tsx`
- `src/components/ui/label.tsx`
- `src/components/ui/Input.tsx`
- `src/components/ui/textarea.tsx`

---

## Verification

1. `npm install` — installs html-to-image
2. `npx tsc --noEmit` — type check passes
3. `cargo tauri dev` — visual check:
   - Bug button captures screenshot, dialog shows preview
   - Both checkboxes unchecked by default, screenshot dimmed
   - "Open GitHub Issue" opens browser with prepopulated title/body
   - Screenshot copied to clipboard when checked (toast confirms)
   - Config is sanitized (no API keys visible in issue body)
   - GitHub button opens repo page
   - Button order: Search, Theme, Bug, GitHub, Help
4. Demo site works with mock `get_config`

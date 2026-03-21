# Plan: Simplify MCP Server Template Configuration

## Context

When adding an MCP server from a template, the user currently lands on a full configuration form with transport type selectors, raw command inputs, auth method dropdowns, env vars, headers, etc. Most of these are pre-determined by the template and shouldn't be editable. This makes adding a well-known template (like GitHub or Time) unnecessarily complex.

Additionally, Stdio templates with `authMethod: 'bearer'` (GitHub, Notion, Brave Search, Cloudflare, Supabase local) are buggy — they show a "Bearer Token" input that creates an `auth_config` stored in keychain, but these servers read API keys from process environment variables, not HTTP bearer tokens. The bearer token is stored but never used.

**Goal**: Template creation should show only the fields that actually need user input. Zero-config templates should be one-click. Templates needing an API key should show just that field.

## Files to Modify

1. `src/components/mcp/McpServerTemplates.tsx` — Add `TemplateField` type, `fields` to templates, fix `authMethod`
2. `src/views/resources/mcp-servers-panel.tsx` — Simplified configure page, field state, updated creation logic

## Implementation

### Step 1: Add `TemplateField` type and `fields` to templates

In `src/components/mcp/McpServerTemplates.tsx`:

Add `TemplateField` interface:
```typescript
export interface TemplateField {
  /** For env_var: the env var key. For arg: used as {{id}} placeholder in args */
  id: string
  label: string
  placeholder: string
  type: 'env_var' | 'arg'
  secret?: boolean
  required?: boolean   // default true
  helpText?: string
  defaultValue?: string
}
```

Add `fields?: TemplateField[]` to `McpServerTemplate`.

### Step 2: Update template definitions

Fix `authMethod` and add `fields` for each template:

| Template | authMethod change | Fields |
|----------|------------------|--------|
| GitHub | `bearer` → `none` | `env: GITHUB_PERSONAL_ACCESS_TOKEN` (secret) |
| Git | — | `arg: repo_path` (replace `/path/to/git/repo` with `{{repo_path}}`) |
| Git MCP Server | — | (none) |
| Notion | `bearer` → `none` | `env: NOTION_TOKEN` (secret) |
| Google Workspace | — | (none) |
| Filesystem | — | `arg: directory` (defaultValue: `{{HOME_DIR}}`) |
| PostgreSQL | — | `arg: connection_url` |
| Supabase (hosted) | — | (none, oauth_browser) |
| Supabase (local) | `bearer` → `none` | `arg: project_ref`, `env: SUPABASE_ACCESS_TOKEN` (secret) |
| Brave Search | `bearer` → `none` | `env: BRAVE_API_KEY` (secret) |
| Fetch | — | (none) |
| AWS Core | — | (none) |
| AWS Docs | — | (none) |
| Google Cloud | — | (none) |
| Kubernetes | — | (none) |
| Docker | — | (none) |
| Cloudflare | `bearer` → `none` | `env: CLOUDFLARE_API_TOKEN` (secret) |
| Time | — | (none) |
| Sequential Thinking | — | (none) |
| Everything Demo | — | (none) |

Args with user-editable values get `{{placeholder}}` syntax:
- Git: `['mcp-server-git', '--repository', '{{repo_path}}']`
- Filesystem: `['-y', '@modelcontextprotocol/server-filesystem', '{{directory}}']`
- PostgreSQL: `['-y', '@modelcontextprotocol/server-postgres', '{{connection_url}}']`
- Supabase local: `['-y', '@supabase/mcp-server-supabase@latest', '--read-only', '--project-ref={{project_ref}}']`

### Step 3: Update `resolveTemplate` to handle field defaultValues

Resolve `HOME_DIR_PLACEHOLDER` in both `args` and field `defaultValue` properties.

### Step 4: Add template field state to mcp-servers-panel

- Add `templateFieldValues` state: `Record<string, string>`
- Update `resetForm` to clear it
- Update `handleSelectTemplate` to initialize field values from `defaultValue`
- Update `initialAddTemplateId` handler similarly

### Step 5: Simplified configure form for templates

When `selectedSource.type === "template"`, render a simplified form instead of the full form:

1. **Header**: Back button + icon + name + description (same as current)
2. **Setup instructions** (if present)
3. **Server Name** input (editable, pre-filled)
4. **Template fields** — dynamically rendered from `template.fields`:
   - Each gets a labeled `Input` with `type="password"` for secrets
   - `helpText` below each field
5. **Command preview** — read-only, shows resolved command with current field values
6. **OAuth note** — for `authMethod: 'oauth_browser'`, a note that browser auth happens after creation
7. **Cancel + Create** buttons

For templates with **no fields** (zero-config), only name + command preview + buttons shown.

Marketplace and custom sources continue to use the full form unchanged.

### Step 6: Update `handleCreateServer` for template fields

When creating from a template with fields:
1. Start with template's `command` and `args`
2. For `type: 'arg'` fields: replace `{{field.id}}` in args with user input
3. For `type: 'env_var'` fields: add to env vars map (skip empty non-required)
4. Join into full command string: `[command, ...resolvedArgs].join(" ")`
5. Build transportConfig: `{ type: "stdio", command: fullCommand, env: envVarsFromFields }`
6. For SSE templates: `{ type: "http_sse", url: template.url, headers: {} }`
7. Auth: only `oauth_browser` sets authConfig; env-var-based auth needs no authConfig

### Step 7: Final steps

1. **Plan review**: Verify all templates have correct fields, no missed edge cases
2. **Test coverage**: Test zero-config, env-var-only, arg-replacement, mixed, oauth_browser, marketplace, custom, and `initialAddTemplateId` paths
3. **Bug hunt**: Check placeholder resolution edge cases, empty field handling, form validation

## Verification

- Select a zero-config template (Time) → only name + command preview shown → Create works
- Select an env-var template (GitHub) → name + token field shown → Create passes token as env var
- Select an arg template (PostgreSQL) → name + connection URL field → arg replaced in command
- Select mixed template (Supabase local) → name + project ref + token fields → both applied
- Select OAuth template (Supabase hosted) → name + oauth note → browser auth after creation
- Custom tab → full form unchanged
- Marketplace → full form unchanged
- Existing server editing (Settings tab) → unchanged

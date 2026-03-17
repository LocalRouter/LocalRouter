# Add Status Page Links to Provider Health Status Card

## Context

The Health Status card in the provider detail page (`providers-panel.tsx`) shows health check results but doesn't link to the provider's public status page. Adding this gives users a quick way to check if an issue is on the provider's end.

## Approach

Frontend-only change to a single file. Add a static map of `provider_type` → status URL, and render a small ExternalLink icon button (with tooltip) in the Health Status card header, next to the existing refresh button. Only shown for providers that have a known status page.

## File to modify

`/Users/matus/dev/localrouterai/src/views/resources/providers-panel.tsx`

## Changes

### 1. Add import for `open` (line 2 area)
```typescript
import { open } from "@tauri-apps/plugin-shell"
```

### 2. Add `ExternalLink` to lucide-react import (line 5)
Append `ExternalLink` to the existing icon import.

### 3. Add status page URL map (after `FREE_TIER_DESCRIPTIONS`, ~line 71)

Keys are verified `provider_type` strings from `crates/lr-providers/src/factory.rs`:

```typescript
const PROVIDER_STATUS_PAGES: Record<string, string> = {
  openai: "https://status.openai.com",
  anthropic: "https://status.anthropic.com",
  gemini: "https://www.google.com/appsstatus/dashboard/",
  mistral: "https://status.mistral.ai",
  cohere: "https://status.cohere.io",
  xai: "https://status.x.ai",
  openrouter: "https://status.openrouter.ai",
  groq: "https://groqstatus.com",
  togetherai: "https://status.together.ai",
  perplexity: "https://status.perplexity.com",
  deepinfra: "https://status.deepinfra.com",
  cerebras: "https://status.cerebras.ai",
}
```

No entry for local providers (ollama, lmstudio, jan, gpt4all, localai, llamacpp) or custom/generic types — they have no public status page.

### 4. Modify Health Status card header (lines 550-565)

Wrap the refresh button in a flex container with `gap-1`, and conditionally add a status page icon button before it:

```tsx
<CardHeader className="pb-3">
  <div className="flex items-center justify-between">
    <CardTitle className="text-sm">Health Status</CardTitle>
    <div className="flex items-center gap-1">
      {PROVIDER_STATUS_PAGES[selectedProvider.provider_type] && (
        <TooltipProvider delayDuration={300}>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="h-6 w-6"
                onClick={() => open(PROVIDER_STATUS_PAGES[selectedProvider.provider_type])}
              >
                <ExternalLink className="h-3 w-3" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Status page</TooltipContent>
          </Tooltip>
        </TooltipProvider>
      )}
      <Button
        variant="ghost"
        size="icon"
        className="h-6 w-6"
        onClick={() => onRefreshHealth(selectedProvider.instance_name)}
        disabled={healthStatus[selectedProvider.instance_name]?.status === "pending"}
      >
        <RefreshCw className={cn(
          "h-3 w-3",
          healthStatus[selectedProvider.instance_name]?.status === "pending" && "animate-spin"
        )} />
      </Button>
    </div>
  </div>
</CardHeader>
```

Reuses existing `Tooltip`/`TooltipProvider`/`TooltipTrigger`/`TooltipContent` already imported at lines 8-12. Button sizing (`h-6 w-6` / `h-3 w-3` icon) matches the adjacent refresh button.

## Verification

1. `npx tsc --noEmit` — no type errors
2. Open a cloud provider detail (e.g. OpenAI) — ExternalLink icon visible in Health Status header
3. Open a local provider detail (e.g. Ollama) — no ExternalLink icon
4. Click the icon — correct status URL opens in system browser
5. Hover the icon — "Status page" tooltip appears

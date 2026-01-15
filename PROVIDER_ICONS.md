# Provider Icons Documentation

## Overview

All provider icons are stored locally in `public/icons/providers/` and are designed for **light backgrounds** (black logos on transparent backgrounds).

## Icon Files

### Official Provider Logos (from LobeHub)
These are high-quality PNG files optimized for light backgrounds:

- **OpenAI** - `openai.png` (46KB)
- **Anthropic/Claude** - `anthropic.png` (45KB)
- **Google Gemini** - `gemini.png` (22KB)
- **Groq** - `groq.png` (24KB)
- **Mistral AI** - `mistral.png` (18KB)
- **Cohere** - `cohere.png` (33KB)
- **Ollama** - `ollama.png` (38KB)
- **Perplexity** - `perplexity.png` (35KB)
- **OpenRouter** - `openrouter.png` (26KB)
- **DeepInfra** - `deepinfra.png` (59KB)
- **Cerebras** - `cerebras.png` (41KB)
- **LM Studio** - `lmstudio.png` (24KB)
- **GitHub** - `github.png` (35KB)

### Custom Icons (created locally)
These are custom-designed icons for providers without official light versions:

- **xAI/Grok** - `xai.png` - Simple X logo in black
- **Together AI** - `togetherai.png` - Network of interconnected dots

## Usage

The `ProviderIcon` component automatically loads icons from `/icons/providers/{provider}.png`:

```tsx
<ProviderIcon providerId="openai" size={32} />
<ProviderIcon providerId="anthropic" size={24} />
```

## Fallback Behavior

If an icon fails to load, the component automatically falls back to emoji:
- OpenAI → 🤖
- Anthropic → 🧠
- Ollama → 🦙
- etc.

## Icon Design Guidelines

All icons follow these principles:
- **Format**: PNG with RGBA transparency
- **Color**: Black (#000000) on transparent background
- **Background**: Optimized for light backgrounds (white/gray)
- **Size**: 256x256 pixels (scales well to any display size)
- **Style**: Professional, clean, recognizable

## Updating Icons

To update or add new icons:

1. **For providers with official logos**:
   - Download from LobeHub CDN light version:
     ```bash
     curl -sL -o {provider}.png "https://registry.npmmirror.com/@lobehub/icons-static-png/latest/files/light/{provider}.png"
     ```

2. **For custom icons**:
   - Create 256x256 PNG with black logo on transparent background
   - Use ImageMagick or similar tool
   - Save to `public/icons/providers/{provider}.png`

3. **Update the component**:
   - Add mapping in `src/components/ProviderIcon.tsx` → `ICON_MAP`
   - Add emoji fallback in `EMOJI_FALLBACK`

## Source

Official icons sourced from: [@lobehub/icons-static-png](https://www.npmjs.com/package/@lobehub/icons-static-png)

## Notes

- All icons are committed to the repository (no external CDN dependencies)
- Icons work offline
- Faster load times compared to CDN
- No network failures
- Consistent appearance across all environments

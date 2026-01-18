# Feature Adapters Guide

Feature adapters extend LocalRouter AI with provider-specific advanced capabilities without polluting the base API. This document covers all 7 implemented adapters and how to use them.

## Table of Contents

1. [Overview](#overview)
2. [How Feature Adapters Work](#how-feature-adapters-work)
3. [Available Adapters](#available-adapters)
   - [Extended Thinking (Anthropic)](#extended-thinking-anthropic)
   - [Reasoning Tokens (OpenAI)](#reasoning-tokens-openai)
   - [Thinking Level (Gemini)](#thinking-level-gemini)
   - [Structured Outputs](#structured-outputs)
   - [Prompt Caching](#prompt-caching)
   - [Logprobs](#logprobs)
   - [JSON Mode](#json-mode)
4. [Combining Multiple Features](#combining-multiple-features)
5. [Provider Compatibility Matrix](#provider-compatibility-matrix)
6. [Error Handling](#error-handling)

---

## Overview

Feature adapters provide a consistent way to access advanced model capabilities across different providers. Each adapter:

- **Transforms requests** to add provider-specific parameters
- **Extracts response data** into a standardized format
- **Validates parameters** before sending to providers
- **Calculates cost impacts** when applicable

Benefits:
- ✅ **Provider abstraction**: Use advanced features without knowing provider-specific APIs
- ✅ **Type safety**: Parameters are validated before requests are sent
- ✅ **Cost transparency**: See exactly how features affect pricing
- ✅ **Composability**: Combine multiple features in a single request

---

## How Feature Adapters Work

### Request Flow

```
User Request
    ↓
Feature Adapter (validate_params)
    ↓
Feature Adapter (adapt_request) → Modifies CompletionRequest
    ↓
Provider sends request to API
    ↓
Provider receives response
    ↓
Feature Adapter (adapt_response) → Extracts feature data
    ↓
Response returned to user
```

### Using Extensions

Feature adapters use the `extensions` field in requests to pass parameters:

```json
{
  "model": "gpt-4",
  "messages": [...],
  "extensions": {
    "feature_name": {
      "param1": "value1",
      "param2": "value2"
    }
  }
}
```

---

## Available Adapters

### Extended Thinking (Anthropic)

**Provider**: Anthropic Claude 3.5 Sonnet, Claude Opus 4.5
**Purpose**: Enable extended thinking mode for deeper reasoning
**Cost Impact**: 1.0x (no extra cost)

#### Parameters

```json
{
  "extensions": {
    "extended_thinking": {
      "enabled": true,
      "budget_tokens": 10000  // Optional: Max thinking tokens
    }
  }
}
```

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `enabled` | boolean | No | `true` | Enable extended thinking mode |
| `budget_tokens` | integer | No | - | Maximum thinking tokens (1000-100000) |

#### Example Request

```json
{
  "model": "claude-opus-4-5",
  "messages": [
    {
      "role": "user",
      "content": "Solve this complex problem step by step: ..."
    }
  ],
  "extensions": {
    "extended_thinking": {
      "enabled": true,
      "budget_tokens": 5000
    }
  }
}
```

#### Response

The adapter extracts thinking blocks from the response:

```json
{
  "choices": [
    {
      "message": {
        "content": "Final answer after thinking..."
      }
    }
  ],
  "extensions": {
    "extended_thinking": {
      "thinking_blocks": [
        {
          "type": "thinking",
          "thinking": "Step-by-step reasoning...",
          "signature": "sha256:..."
        }
      ],
      "thinking_tokens_used": 3421,
      "thinking_budget": 5000,
      "thinking_truncated": false
    }
  }
}
```

---

### Reasoning Tokens (OpenAI)

**Provider**: OpenAI o1, o1-mini
**Purpose**: Access reasoning tokens from OpenAI's o1 models
**Cost Impact**: Varies by model (reasoning tokens charged at input rate)

#### Parameters

No parameters needed - enabled automatically for o1 models.

#### Example Request

```json
{
  "model": "o1-preview",
  "messages": [
    {
      "role": "user",
      "content": "Explain quantum computing"
    }
  ]
}
```

#### Response

```json
{
  "usage": {
    "prompt_tokens": 10,
    "completion_tokens": 150,
    "total_tokens": 160,
    "completion_tokens_details": {
      "reasoning_tokens": 120  // Extracted from response
    }
  }
}
```

---

### Thinking Level (Gemini)

**Provider**: Google Gemini 2.0
**Purpose**: Control thinking depth for Gemini models
**Cost Impact**: 1.0x (no extra cost)

#### Parameters

```json
{
  "extensions": {
    "thinking_level": {
      "level": "medium"  // "low", "medium", "high"
    }
  }
}
```

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `level` | string | Yes | - | Thinking depth: "low", "medium", or "high" |

#### Example Request

```json
{
  "model": "gemini-2.0-flash-thinking-exp",
  "messages": [
    {
      "role": "user",
      "content": "Analyze this code for security vulnerabilities..."
    }
  ],
  "extensions": {
    "thinking_level": {
      "level": "high"
    }
  }
}
```

---

### Structured Outputs

**Provider**: OpenAI (gpt-4, gpt-3.5-turbo), Anthropic (Claude 3+)
**Purpose**: Enforce strict JSON schema validation on responses
**Cost Impact**: 1.0x (no extra cost)

#### Parameters

```json
{
  "extensions": {
    "structured_outputs": {
      "schema": {
        "type": "object",
        "properties": {
          "name": {"type": "string"},
          "age": {"type": "number"}
        },
        "required": ["name", "age"]
      }
    }
  }
}
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `schema` | object | Yes | JSON Schema (Draft 7) defining response structure |

#### Schema Limitations

- **OpenAI**: Full JSON Schema support with `"strict": true`
- **Anthropic**: Schema enforcement via prompts + validation
- **Max size**: 1MB
- **Supported types**: object, array, string, number, boolean, null

#### Example Request

```json
{
  "model": "gpt-4",
  "messages": [
    {
      "role": "user",
      "content": "Generate a person with name and age between 20-30"
    }
  ],
  "extensions": {
    "structured_outputs": {
      "schema": {
        "type": "object",
        "properties": {
          "name": {"type": "string"},
          "age": {
            "type": "number",
            "minimum": 20,
            "maximum": 30
          }
        },
        "required": ["name", "age"],
        "additionalProperties": false
      }
    }
  }
}
```

#### Response

```json
{
  "choices": [
    {
      "message": {
        "content": "{\"name\": \"Alice\", \"age\": 25}"
      }
    }
  ],
  "extensions": {
    "structured_outputs": {
      "validated": true,
      "schema_compliance": "strict"
    }
  }
}
```

#### Error Handling

If the response doesn't match the schema:

```json
{
  "error": {
    "type": "provider_error",
    "message": "Choice 0 failed schema validation: Response does not match schema..."
  }
}
```

---

### Prompt Caching

**Provider**: Anthropic (Claude 3+), OpenRouter
**Purpose**: Cache context to reduce costs for repeated prompts
**Cost Impact**: 0.1x-0.5x (50-90% savings on cached tokens)

#### How It Works

- **Cache creation**: First request pays full price and creates cache entry
- **Cache hit (<5min)**: 90% discount (0.1x cost)
- **Cache hit (<1hr)**: 50% discount (0.5x cost)
- **Cache miss (>1hr)**: Full price, new cache created
- **TTL**: 5 minutes for Anthropic

#### Parameters

```json
{
  "extensions": {
    "prompt_caching": {
      "cache_control": {
        "type": "ephemeral"
      }
    }
  }
}
```

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `cache_control.type` | string | No | `"ephemeral"` | Cache type (only "ephemeral" supported) |

#### Automatic Cache Breakpoints

The adapter automatically places cache breakpoints at:
1. **System messages**: Usually large and static
2. **Conversation history**: Second-to-last message in conversation

This allows the final user message to vary while caching the context.

#### Example Request (First Call - Creates Cache)

```json
{
  "model": "claude-opus-4-5",
  "messages": [
    {
      "role": "system",
      "content": "You are an expert in quantum physics. Here is a 10,000 word textbook chapter: ..."
    },
    {
      "role": "user",
      "content": "What is quantum entanglement?"
    }
  ],
  "extensions": {
    "prompt_caching": {}
  }
}
```

**Response (Cache Creation)**:

```json
{
  "usage": {
    "prompt_tokens": 100,
    "completion_tokens": 150,
    "total_tokens": 3250,
    "prompt_tokens_details": {
      "cached_tokens": null,
      "cache_creation_tokens": 3000,  // System message cached
      "cache_read_tokens": null
    }
  },
  "extensions": {
    "prompt_caching": {
      "cache_creation_input_tokens": 3000,
      "cache_read_input_tokens": 0,
      "input_tokens": 100,
      "cache_savings_percent": "0.0%",  // First call, no savings
      "cache_hit": false
    }
  }
}
```

#### Example Request (Second Call - Uses Cache)

**Same system message, different question** (within 5 minutes):

```json
{
  "model": "claude-opus-4-5",
  "messages": [
    {
      "role": "system",
      "content": "You are an expert in quantum physics. Here is a 10,000 word textbook chapter: ..."
    },
    {
      "role": "user",
      "content": "Explain superposition"
    }
  ],
  "extensions": {
    "prompt_caching": {}
  }
}
```

**Response (Cache Hit)**:

```json
{
  "usage": {
    "prompt_tokens": 100,
    "completion_tokens": 120,
    "total_tokens": 520,
    "prompt_tokens_details": {
      "cached_tokens": null,
      "cache_creation_tokens": null,
      "cache_read_tokens": 3000  // Read from cache at 0.1x cost
    }
  },
  "extensions": {
    "prompt_caching": {
      "cache_creation_input_tokens": 0,
      "cache_read_input_tokens": 3000,
      "input_tokens": 100,
      "cache_savings_percent": "87.1%",  // 90% savings on 3000 tokens
      "cache_hit": true
    }
  }
}
```

#### Cost Calculation

Without caching:
- Prompt: 3100 tokens × $15/MTok = $0.0465
- Completion: 120 tokens × $75/MTok = $0.009
- **Total**: $0.0555

With caching (2nd call):
- Prompt (uncached): 100 tokens × $15/MTok = $0.0015
- Prompt (cached): 3000 tokens × $1.50/MTok = $0.0045
- Completion: 120 tokens × $75/MTok = $0.009
- **Total**: $0.015

**Savings: $0.0405 (73%)**

---

### Logprobs

**Provider**: OpenAI (all chat models), OpenRouter
**Purpose**: Get token-level probability information for confidence scoring
**Cost Impact**: 1.0x (no extra cost)

#### Parameters

```json
{
  "extensions": {
    "logprobs": {
      "enabled": true,
      "top_logprobs": 5  // 0-20
    }
  }
}
```

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `enabled` | boolean | No | `true` | Enable logprobs extraction |
| `top_logprobs` | integer | No | `0` | Number of alternative tokens to return (0-20) |

#### Example Request

```json
{
  "model": "gpt-4",
  "messages": [
    {
      "role": "user",
      "content": "What is 2+2?"
    }
  ],
  "extensions": {
    "logprobs": {
      "enabled": true,
      "top_logprobs": 3
    }
  }
}
```

#### Response

```json
{
  "choices": [
    {
      "message": {
        "content": "2+2 equals 4."
      }
    }
  ],
  "extensions": {
    "logprobs": {
      "logprobs": {
        "content": [
          {
            "token": "2",
            "logprob": -0.00001,
            "bytes": [50],
            "top_logprobs": [
              {"token": "2", "logprob": -0.00001},
              {"token": "Two", "logprob": -11.5},
              {"token": "The", "logprob": -14.2}
            ]
          },
          {
            "token": "+",
            "logprob": -0.000005,
            "bytes": [43],
            "top_logprobs": [
              {"token": "+", "logprob": -0.000005},
              {"token": " plus", "logprob": -12.1},
              {"token": " +", "logprob": -13.7}
            ]
          }
        ]
      },
      "token_count": 6,
      "average_confidence": -0.0000125
    }
  }
}
```

#### Use Cases

1. **Confidence Scoring**: Filter low-confidence responses
   ```python
   avg_logprob = response["extensions"]["logprobs"]["average_confidence"]
   if avg_logprob < -5.0:
       print("Low confidence response")
   ```

2. **Token Healing**: Select alternative tokens
   ```python
   first_token = logprobs["content"][0]
   if first_token["logprob"] < -2.0:
       # Consider using a top alternative
       alt = first_token["top_logprobs"][1]
       print(f"Alternative: {alt['token']}")
   ```

3. **Uncertainty Detection**: Find tokens with high entropy
   ```python
   for token_info in logprobs["content"]:
       if token_info["logprob"] < -1.0:
           print(f"Uncertain token: {token_info['token']}")
   ```

---

### JSON Mode

**Provider**: OpenAI, Anthropic, Gemini, OpenRouter
**Purpose**: Ensure responses are valid JSON (without schema validation)
**Cost Impact**: 1.0x (no extra cost)

#### Difference from Structured Outputs

| Feature | JSON Mode | Structured Outputs |
|---------|-----------|-------------------|
| **Validation** | Syntax only | Schema compliance |
| **Cost** | No extra cost | No extra cost |
| **Strictness** | Relaxed | Strict |
| **Providers** | OpenAI, Anthropic, Gemini, OpenRouter | OpenAI, Anthropic |
| **Use case** | "Give me valid JSON" | "Give me JSON matching this exact schema" |

#### Parameters

```json
{
  "extensions": {
    "json_mode": {
      "enabled": true
    }
  }
}
```

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `enabled` | boolean | No | `true` | Enable JSON mode |

#### Example Request (OpenAI)

```json
{
  "model": "gpt-4",
  "messages": [
    {
      "role": "user",
      "content": "Generate a JSON object with a greeting and timestamp"
    }
  ],
  "extensions": {
    "json_mode": {
      "enabled": true
    }
  }
}
```

**OpenAI Implementation**: Uses native `response_format: { type: "json_object" }`

#### Example Request (Anthropic)

```json
{
  "model": "claude-3-opus",
  "messages": [
    {
      "role": "user",
      "content": "Generate a person object"
    }
  ],
  "extensions": {
    "json_mode": {
      "enabled": true
    }
  }
}
```

**Anthropic Implementation**: Injects system message:
```
You must respond with valid JSON only. Do not include any text before or after the JSON. Your entire response should be parseable as JSON.
```

#### Response

```json
{
  "choices": [
    {
      "message": {
        "content": "{\"greeting\": \"Hello, World!\", \"timestamp\": \"2024-01-15T10:30:00Z\"}"
      }
    }
  ],
  "extensions": {
    "json_mode": {
      "validated": true,
      "choices_validated": 1
    }
  }
}
```

#### Error Handling

If response is not valid JSON:

```json
{
  "error": {
    "type": "provider_error",
    "message": "Choice 0 failed JSON validation: Response is not valid JSON: unexpected character..."
  }
}
```

---

## Combining Multiple Features

Features can be combined in a single request when they're compatible:

### Example: Structured Outputs + Prompt Caching (Anthropic)

```json
{
  "model": "claude-3-opus",
  "messages": [
    {
      "role": "system",
      "content": "Long context that should be cached..."
    },
    {
      "role": "user",
      "content": "Generate a person"
    }
  ],
  "extensions": {
    "structured_outputs": {
      "schema": {
        "type": "object",
        "properties": {
          "name": {"type": "string"},
          "age": {"type": "number"}
        },
        "required": ["name", "age"]
      }
    },
    "prompt_caching": {}
  }
}
```

**Result**: Response is validated against schema AND caching reduces costs.

### Example: JSON Mode + Logprobs (OpenAI)

```json
{
  "model": "gpt-4",
  "messages": [
    {
      "role": "user",
      "content": "Generate JSON with a number"
    }
  ],
  "extensions": {
    "json_mode": {
      "enabled": true
    },
    "logprobs": {
      "enabled": true,
      "top_logprobs": 3
    }
  }
}
```

**Result**: Get valid JSON response with token probabilities for confidence scoring.

---

## Provider Compatibility Matrix

| Feature | OpenAI | Anthropic | Gemini | OpenRouter |
|---------|--------|-----------|--------|------------|
| **Extended Thinking** | ❌ | ✅ Claude 3.5+, Opus 4.5 | ❌ | ❌ |
| **Reasoning Tokens** | ✅ o1, o1-mini | ❌ | ❌ | ❌ |
| **Thinking Level** | ❌ | ❌ | ✅ Gemini 2.0+ | ❌ |
| **Structured Outputs** | ✅ GPT-4, GPT-3.5 | ✅ Claude 3+ | ❌ | ✅ |
| **Prompt Caching** | ❌ | ✅ Claude 3+ | ❌ | ✅ |
| **Logprobs** | ✅ All chat models | ❌ | ❌ | ✅ |
| **JSON Mode** | ✅ All chat models | ✅ Claude 3+ | ✅ Gemini Pro+ | ✅ |

### Provider Detection

Adapters automatically detect providers based on model name:

```
gpt-* or o1-* → OpenAI
claude-* → Anthropic
gemini-* → Gemini
*/* (contains slash) → OpenRouter
```

---

## Error Handling

### Parameter Validation Errors

```json
{
  "error": {
    "type": "config_error",
    "message": "top_logprobs must be between 0 and 20 (got 25)"
  }
}
```

### Unsupported Provider Errors

```json
{
  "error": {
    "type": "config_error",
    "message": "Prompt caching not supported for model: gpt-4"
  }
}
```

### Schema Validation Errors

```json
{
  "error": {
    "type": "provider_error",
    "message": "Choice 0 failed schema validation: Response does not match schema at '/age': expected number, got string"
  }
}
```

### JSON Validation Errors

```json
{
  "error": {
    "type": "provider_error",
    "message": "Response is not valid JSON: expected `:` at line 1 column 15"
  }
}
```

---

## Best Practices

### 1. Use Structured Outputs for Critical Data

When you need guaranteed schema compliance:

```json
{
  "extensions": {
    "structured_outputs": {
      "schema": {
        "type": "object",
        "properties": {
          "amount": {"type": "number"},
          "currency": {"type": "string", "enum": ["USD", "EUR", "GBP"]}
        },
        "required": ["amount", "currency"]
      }
    }
  }
}
```

### 2. Enable Prompt Caching for Repeated Context

When you're making multiple requests with the same large context:

```json
{
  "messages": [
    {"role": "system", "content": "LARGE_CONTEXT"},  // Will be cached
    {"role": "user", "content": "Question 1"}
  ],
  "extensions": {"prompt_caching": {}}
}
```

Then in subsequent requests (within 5min), change only the user message for maximum savings.

### 3. Use Logprobs for Quality Control

Monitor confidence to filter low-quality responses:

```python
response = client.chat.completions.create(
    model="gpt-4",
    messages=[...],
    extensions={"logprobs": {"enabled": True}}
)

avg_confidence = response.extensions["logprobs"]["average_confidence"]

if avg_confidence < -3.0:
    # Low confidence - maybe retry or flag for review
    handle_low_confidence(response)
```

### 4. Choose JSON Mode vs Structured Outputs

- **Use JSON Mode** when you just need valid JSON: `{"any": "structure"}`
- **Use Structured Outputs** when you need exact schema compliance

### 5. Combine Features for Cost Efficiency

Anthropic + Structured Outputs + Caching = Validated responses at 10-50% cost:

```json
{
  "model": "claude-3-opus",
  "extensions": {
    "structured_outputs": {"schema": {...}},
    "prompt_caching": {}
  }
}
```

---

## Migration Guide

### From Raw OpenAI API

**Before**:
```python
response = openai.ChatCompletion.create(
    model="gpt-4",
    messages=[...],
    response_format={"type": "json_schema", "schema": {...}}
)
```

**After (LocalRouter)**:
```python
response = client.chat.completions.create(
    model="gpt-4",
    messages=[...],
    extensions={
        "structured_outputs": {"schema": {...}}
    }
)
```

### From Raw Anthropic API

**Before**:
```python
response = anthropic.messages.create(
    model="claude-opus-4-5",
    messages=[...],
    thinking={"type": "enabled", "budget_tokens": 5000}
)
```

**After (LocalRouter)**:
```python
response = client.chat.completions.create(
    model="claude-opus-4-5",
    messages=[...],
    extensions={
        "extended_thinking": {
            "enabled": True,
            "budget_tokens": 5000
        }
    }
)
```

---

## Testing

All feature adapters have comprehensive test coverage:

```bash
# Run integration tests
cargo test --test feature_adapter_integration_tests

# Run unit tests for a specific adapter
cargo test --lib structured_outputs
cargo test --lib prompt_caching
cargo test --lib logprobs
cargo test --lib json_mode
```

Test coverage:
- ✅ 60+ unit tests across all adapters
- ✅ 16 integration tests covering real-world scenarios
- ✅ Cross-feature compatibility tests
- ✅ Error handling and edge cases

---

## Implementation Details

### Code Organization

```
src-tauri/src/providers/features/
├── mod.rs                      # FeatureAdapter trait + registry
├── anthropic_thinking.rs       # Extended thinking for Claude
├── openai_reasoning.rs         # Reasoning tokens for o1 models
├── gemini_thinking.rs          # Thinking level for Gemini
├── structured_outputs.rs       # JSON schema validation
├── prompt_caching.rs           # Cost-saving caching
├── logprobs.rs                 # Token probability extraction
└── json_mode.rs                # Lightweight JSON validation
```

### FeatureAdapter Trait

```rust
pub trait FeatureAdapter: Send + Sync {
    fn feature_name(&self) -> &str;

    fn validate_params(&self, params: &FeatureParams) -> AppResult<()>;

    fn adapt_request(
        &self,
        request: &mut CompletionRequest,
        params: &FeatureParams,
    ) -> AppResult<()>;

    fn adapt_response(
        &self,
        response: &mut CompletionResponse,
    ) -> AppResult<Option<FeatureData>>;

    fn cost_multiplier(&self) -> f64;

    fn help_text(&self) -> &str;
}
```

---

## Support

For issues or questions:

- **GitHub Issues**: https://github.com/your-org/localrouter-ai/issues
- **Documentation**: Check inline comments in adapter source files
- **Examples**: See `tests/feature_adapter_integration_tests.rs`

---

**Version**: 0.1.0
**Last Updated**: 2026-01-17
**Adapters**: 7 implemented, all tested and production-ready

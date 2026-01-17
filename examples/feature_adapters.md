# Feature Adapters - Quick Reference & Examples

This guide provides copy-paste examples for all 7 feature adapters in LocalRouter AI.

## Quick Navigation

- [Structured Outputs](#structured-outputs)
- [Prompt Caching](#prompt-caching)
- [Logprobs](#logprobs)
- [JSON Mode](#json-mode)
- [Extended Thinking](#extended-thinking)
- [Reasoning Tokens](#reasoning-tokens)
- [Thinking Level](#thinking-level)

---

## Structured Outputs

**When to use**: Need responses to match an exact JSON schema

### Example 1: Generate Person with Constraints

```bash
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4",
    "messages": [
      {
        "role": "user",
        "content": "Generate a random person"
      }
    ],
    "extensions": {
      "structured_outputs": {
        "schema": {
          "type": "object",
          "properties": {
            "name": {
              "type": "string",
              "minLength": 1
            },
            "age": {
              "type": "number",
              "minimum": 0,
              "maximum": 120
            },
            "email": {
              "type": "string",
              "format": "email"
            },
            "isActive": {
              "type": "boolean"
            }
          },
          "required": ["name", "age", "email"],
          "additionalProperties": false
        }
      }
    }
  }'
```

**Expected Response**:
```json
{
  "choices": [
    {
      "message": {
        "content": "{\"name\":\"Alice Johnson\",\"age\":28,\"email\":\"alice@example.com\",\"isActive\":true}"
      }
    }
  ]
}
```

### Example 2: Extract Data with Nested Objects

```json
{
  "model": "gpt-4",
  "messages": [
    {
      "role": "user",
      "content": "Extract information from this text: 'John Doe, age 35, lives in New York, works as a Software Engineer at TechCorp'"
    }
  ],
  "extensions": {
    "structured_outputs": {
      "schema": {
        "type": "object",
        "properties": {
          "person": {
            "type": "object",
            "properties": {
              "name": {"type": "string"},
              "age": {"type": "number"}
            },
            "required": ["name", "age"]
          },
          "location": {
            "type": "object",
            "properties": {
              "city": {"type": "string"},
              "country": {"type": "string"}
            },
            "required": ["city"]
          },
          "job": {
            "type": "object",
            "properties": {
              "title": {"type": "string"},
              "company": {"type": "string"}
            },
            "required": ["title"]
          }
        },
        "required": ["person", "location", "job"]
      }
    }
  }
}
```

### Example 3: Generate Array with Enums

```json
{
  "model": "claude-3-opus",
  "messages": [
    {
      "role": "user",
      "content": "Generate 3 products"
    }
  ],
  "extensions": {
    "structured_outputs": {
      "schema": {
        "type": "object",
        "properties": {
          "products": {
            "type": "array",
            "items": {
              "type": "object",
              "properties": {
                "id": {"type": "string"},
                "name": {"type": "string"},
                "price": {"type": "number", "minimum": 0},
                "category": {
                  "type": "string",
                  "enum": ["Electronics", "Clothing", "Food", "Books"]
                },
                "inStock": {"type": "boolean"}
              },
              "required": ["id", "name", "price", "category", "inStock"]
            },
            "minItems": 1,
            "maxItems": 10
          }
        },
        "required": ["products"]
      }
    }
  }
}
```

---

## Prompt Caching

**When to use**: Making multiple requests with large shared context (e.g., long documents, code files)

### Example 1: Q&A on Long Document

**First Request** (creates cache):

```bash
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-opus-4-5",
    "messages": [
      {
        "role": "system",
        "content": "You are a legal expert. Here is a 50-page contract: [FULL CONTRACT TEXT...]"
      },
      {
        "role": "user",
        "content": "What are the termination clauses?"
      }
    ],
    "extensions": {
      "prompt_caching": {}
    }
  }'
```

**Response**: Shows cache creation
```json
{
  "usage": {
    "prompt_tokens": 200,
    "completion_tokens": 150,
    "total_tokens": 12350,
    "prompt_tokens_details": {
      "cache_creation_tokens": 12000,  // Contract cached
      "cache_read_tokens": null
    }
  },
  "extensions": {
    "prompt_caching": {
      "cache_creation_input_tokens": 12000,
      "cache_read_input_tokens": 0,
      "cache_savings_percent": "0.0%",
      "cache_hit": false
    }
  }
}
```

**Follow-up Request** (within 5 minutes):

```json
{
  "model": "claude-opus-4-5",
  "messages": [
    {
      "role": "system",
      "content": "You are a legal expert. Here is a 50-page contract: [SAME CONTRACT TEXT...]"
    },
    {
      "role": "user",
      "content": "What are the payment terms?"  // Different question
    }
  ],
  "extensions": {
    "prompt_caching": {}
  }
}
```

**Response**: Shows cache hit and savings
```json
{
  "usage": {
    "prompt_tokens": 200,
    "completion_tokens": 120,
    "total_tokens": 12320,
    "prompt_tokens_details": {
      "cache_creation_tokens": null,
      "cache_read_tokens": 12000  // Read from cache
    }
  },
  "extensions": {
    "prompt_caching": {
      "cache_creation_input_tokens": 0,
      "cache_read_input_tokens": 12000,
      "cache_savings_percent": "88.2%",  // 90% savings on 12000 tokens
      "cache_hit": true
    }
  }
}
```

### Example 2: Code Analysis with Caching

```json
{
  "model": "claude-3-opus",
  "messages": [
    {
      "role": "system",
      "content": "You are a senior code reviewer. Here is the codebase: [LARGE CODEBASE...]"
    },
    {
      "role": "user",
      "content": "user.py",
      "role": "assistant",
      "content": "Here's the security analysis..."
    },
    {
      "role": "user",
      "content": "Now analyze database.py"
    }
  ],
  "extensions": {
    "prompt_caching": {}
  }
}
```

**Caching Strategy**:
- System message (codebase) is cached
- Conversation history up to second-to-last message is cached
- Only the final question varies

---

## Logprobs

**When to use**: Need to measure model confidence or explore alternative tokens

### Example 1: Confidence Scoring

```bash
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4",
    "messages": [
      {
        "role": "user",
        "content": "What is the capital of France?"
      }
    ],
    "extensions": {
      "logprobs": {
        "enabled": true,
        "top_logprobs": 3
      }
    }
  }'
```

**Response**:
```json
{
  "choices": [
    {
      "message": {
        "content": "The capital of France is Paris."
      }
    }
  ],
  "extensions": {
    "logprobs": {
      "logprobs": {
        "content": [
          {
            "token": "The",
            "logprob": -0.00001,
            "bytes": [84, 104, 101],
            "top_logprobs": [
              {"token": "The", "logprob": -0.00001},
              {"token": "Paris", "logprob": -11.5},
              {"token": "France", "logprob": -12.1}
            ]
          },
          {
            "token": " capital",
            "logprob": -0.000005,
            "bytes": [32, 99, 97, 112, 105, 116, 97, 108],
            "top_logprobs": [
              {"token": " capital", "logprob": -0.000005},
              {"token": " answer", "logprob": -9.2},
              {"token": " city", "logprob": -10.5}
            ]
          }
        ]
      },
      "token_count": 8,
      "average_confidence": -0.0000025  // Very high confidence
    }
  }
}
```

**Interpreting Confidence**:
- `-0.01` to `0`: Extremely confident (>99% probability)
- `-1` to `-0.01`: Very confident (>90% probability)
- `-3` to `-1`: Confident (>75% probability)
- `< -5`: Low confidence (<50% probability)

### Example 2: Detect Uncertain Responses

```python
import requests

response = requests.post('http://localhost:3000/v1/chat/completions', json={
    "model": "gpt-4",
    "messages": [
        {"role": "user", "content": "Will it rain tomorrow in London?"}
    ],
    "extensions": {
        "logprobs": {
            "enabled": True,
            "top_logprobs": 5
        }
    }
})

data = response.json()
avg_confidence = data["extensions"]["logprobs"]["average_confidence"]

if avg_confidence < -2.0:
    print("⚠️ Model is uncertain about this answer")
    print("Consider asking for clarification or additional context")
else:
    print("✅ Model is confident in the answer")
```

### Example 3: Token Healing

```python
logprobs = response["extensions"]["logprobs"]["logprobs"]["content"]

# Check first token confidence
first_token = logprobs[0]
if first_token["logprob"] < -1.0:
    print(f"Primary token: {first_token['token']} (low confidence)")
    print("Alternatives:")
    for alt in first_token["top_logprobs"]:
        print(f"  - {alt['token']}: {alt['logprob']}")
```

---

## JSON Mode

**When to use**: Need valid JSON without strict schema requirements

### Example 1: OpenAI JSON Mode

```bash
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4",
    "messages": [
      {
        "role": "user",
        "content": "Generate a random object with any fields you want"
      }
    ],
    "extensions": {
      "json_mode": {
        "enabled": true
      }
    }
  }'
```

**Response** (guaranteed valid JSON):
```json
{
  "choices": [
    {
      "message": {
        "content": "{\"id\":\"abc123\",\"type\":\"random\",\"value\":42,\"metadata\":{\"generated\":true}}"
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

### Example 2: Anthropic JSON Mode

```json
{
  "model": "claude-3-opus",
  "messages": [
    {
      "role": "user",
      "content": "List 3 fruits with their colors in JSON format"
    }
  ],
  "extensions": {
    "json_mode": {
      "enabled": true
    }
  }
}
```

**What happens**:
- System message is automatically injected: "You must respond with valid JSON only..."
- Response is validated to ensure it's parseable JSON
- No schema enforcement (any valid JSON structure is accepted)

### Example 3: JSON Mode vs Structured Outputs

```python
# JSON Mode: Any valid JSON
json_mode_request = {
    "extensions": {
        "json_mode": {"enabled": True}
    }
}
# ✅ Accepts: {"any": "structure"}
# ✅ Accepts: [1, 2, 3]
# ✅ Accepts: {"nested": {"objects": true}}

# Structured Outputs: Must match schema
structured_request = {
    "extensions": {
        "structured_outputs": {
            "schema": {
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"]
            }
        }
    }
}
# ✅ Accepts: {"name": "Alice"}
# ❌ Rejects: {"name": 123}  (wrong type)
# ❌ Rejects: {}  (missing required field)
```

---

## Extended Thinking

**When to use**: Complex reasoning tasks with Anthropic Claude

### Example 1: Math Problem Solving

```bash
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-opus-4-5",
    "messages": [
      {
        "role": "user",
        "content": "A train leaves Station A at 60 mph heading east. Another train leaves Station B (120 miles east of A) at 40 mph heading west. When do they meet?"
      }
    ],
    "extensions": {
      "extended_thinking": {
        "enabled": true,
        "budget_tokens": 5000
      }
    }
  }'
```

**Response**:
```json
{
  "choices": [
    {
      "message": {
        "content": "The trains will meet after 1.2 hours (72 minutes)..."
      }
    }
  ],
  "extensions": {
    "extended_thinking": {
      "thinking_blocks": [
        {
          "type": "thinking",
          "thinking": "Let me work through this step by step:\n1. Train A: 60 mph east\n2. Train B: 40 mph west\n3. Combined closing speed: 60 + 40 = 100 mph\n4. Distance: 120 miles\n5. Time = Distance / Speed = 120 / 100 = 1.2 hours",
          "signature": "sha256:..."
        }
      ],
      "thinking_tokens_used": 142,
      "thinking_budget": 5000,
      "thinking_truncated": false
    }
  }
}
```

### Example 2: Code Analysis

```json
{
  "model": "claude-opus-4-5",
  "messages": [
    {
      "role": "user",
      "content": "Find all security vulnerabilities in this code: [LARGE CODE SNIPPET]"
    }
  ],
  "extensions": {
    "extended_thinking": {
      "enabled": true,
      "budget_tokens": 10000
    }
  }
}
```

**Use Cases**:
- Mathematical problem solving
- Code review and bug detection
- Legal document analysis
- Complex reasoning tasks
- Multi-step planning

---

## Reasoning Tokens

**When to use**: Using OpenAI o1 models

### Example 1: O1 Reasoning

```bash
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "o1-preview",
    "messages": [
      {
        "role": "user",
        "content": "Explain how neural networks learn through backpropagation"
      }
    ]
  }'
```

**Response** (automatic extraction):
```json
{
  "usage": {
    "prompt_tokens": 15,
    "completion_tokens": 450,
    "total_tokens": 465,
    "completion_tokens_details": {
      "reasoning_tokens": 350  // Automatically extracted
    }
  }
}
```

**Note**: No configuration needed - reasoning tokens are automatically extracted from o1 models.

---

## Thinking Level

**When to use**: Controlling reasoning depth in Gemini 2.0

### Example 1: High Thinking for Complex Tasks

```bash
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gemini-2.0-flash-thinking-exp",
    "messages": [
      {
        "role": "user",
        "content": "Design a scalable microservices architecture for an e-commerce platform"
      }
    ],
    "extensions": {
      "thinking_level": {
        "level": "high"
      }
    }
  }'
```

### Example 2: Low Thinking for Simple Tasks

```json
{
  "model": "gemini-2.0-flash-thinking-exp",
  "messages": [
    {
      "role": "user",
      "content": "What is 5 + 3?"
    }
  ],
  "extensions": {
    "thinking_level": {
      "level": "low"
    }
  }
}
```

**Thinking Levels**:
- `"low"`: Quick responses for simple queries
- `"medium"`: Balanced reasoning for general tasks
- `"high"`: Deep thinking for complex problems

---

## Combined Features

### Example 1: Structured Outputs + Caching (Cost-Optimized Data Extraction)

```json
{
  "model": "claude-3-opus",
  "messages": [
    {
      "role": "system",
      "content": "You extract data from invoices. Here are 50 example invoices: [EXAMPLES...]"
    },
    {
      "role": "user",
      "content": "Extract data from this invoice: [INVOICE TEXT]"
    }
  ],
  "extensions": {
    "structured_outputs": {
      "schema": {
        "type": "object",
        "properties": {
          "invoice_number": {"type": "string"},
          "date": {"type": "string", "format": "date"},
          "total": {"type": "number"},
          "items": {
            "type": "array",
            "items": {
              "type": "object",
              "properties": {
                "description": {"type": "string"},
                "quantity": {"type": "number"},
                "price": {"type": "number"}
              },
              "required": ["description", "quantity", "price"]
            }
          }
        },
        "required": ["invoice_number", "date", "total", "items"]
      }
    },
    "prompt_caching": {}
  }
}
```

**Benefits**:
- ✅ Validated extraction (schema enforcement)
- ✅ 90% cost savings on examples (caching)
- ✅ Fast subsequent requests

### Example 2: JSON Mode + Logprobs (Confidence-Scored JSON)

```json
{
  "model": "gpt-4",
  "messages": [
    {
      "role": "user",
      "content": "Is this email spam? Respond with JSON"
    }
  ],
  "extensions": {
    "json_mode": {"enabled": true},
    "logprobs": {
      "enabled": true,
      "top_logprobs": 3
    }
  }
}
```

**Benefits**:
- ✅ Guaranteed valid JSON
- ✅ Confidence scores for each token
- ✅ Can detect uncertain classifications

---

## Testing Examples

Run the integration tests to see all features in action:

```bash
# Run all feature adapter tests
cargo test --test feature_adapter_integration_tests

# Run specific test
cargo test --test feature_adapter_integration_tests test_structured_outputs_person_schema_openai
```

---

## Python Client Examples

```python
import requests

BASE_URL = "http://localhost:3000/v1/chat/completions"
API_KEY = "your-api-key"

def structured_output_example():
    response = requests.post(BASE_URL, json={
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Generate a person"}],
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
    }, headers={"Authorization": f"Bearer {API_KEY}"})

    return response.json()

def caching_example():
    # First request (creates cache)
    response1 = requests.post(BASE_URL, json={
        "model": "claude-opus-4-5",
        "messages": [
            {"role": "system", "content": "LARGE_CONTEXT"},
            {"role": "user", "content": "Question 1"}
        ],
        "extensions": {"prompt_caching": {}}
    }, headers={"Authorization": f"Bearer {API_KEY}"})

    # Second request (uses cache)
    response2 = requests.post(BASE_URL, json={
        "model": "claude-opus-4-5",
        "messages": [
            {"role": "system", "content": "LARGE_CONTEXT"},  # Same context
            {"role": "user", "content": "Question 2"}  # Different question
        ],
        "extensions": {"prompt_caching": {}}
    }, headers={"Authorization": f"Bearer {API_KEY}"})

    savings = response2.json()["extensions"]["prompt_caching"]["cache_savings_percent"]
    print(f"Cache savings: {savings}")
```

---

**Version**: 0.1.0
**Last Updated**: 2026-01-17

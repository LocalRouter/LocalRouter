#!/bin/bash
# Test embeddings endpoint

BASE_URL="http://localhost:3625"
API_KEY="test-key"

echo "Testing OpenAI embeddings endpoint..."
echo ""

# Test 1: Single text input
echo "Test 1: Single text input"
curl -s -X POST "${BASE_URL}/v1/embeddings" \
  -H "Authorization: Bearer ${API_KEY}" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "openai/text-embedding-ada-002",
    "input": "Hello world"
  }' | jq '.'

echo ""
echo "---"
echo ""

# Test 2: Multiple text inputs
echo "Test 2: Multiple text inputs"
curl -s -X POST "${BASE_URL}/v1/embeddings" \
  -H "Authorization: Bearer ${API_KEY}" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "openai/text-embedding-ada-002",
    "input": ["Hello world", "How are you?", "Testing embeddings"]
  }' | jq '.'

echo ""
echo "---"
echo ""

# Test 3: Ollama embeddings (if available)
echo "Test 3: Ollama embeddings"
curl -s -X POST "${BASE_URL}/v1/embeddings" \
  -H "Authorization: Bearer ${API_KEY}" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "ollama/nomic-embed-text",
    "input": "Testing Ollama embeddings"
  }' | jq '.'

echo ""
echo "---"
echo ""

# Test 4: Cohere embeddings (if API key is configured)
echo "Test 4: Cohere embeddings"
curl -s -X POST "${BASE_URL}/v1/embeddings" \
  -H "Authorization: Bearer ${API_KEY}" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "cohere/embed-english-v3.0",
    "input": "Testing Cohere embeddings"
  }' | jq '.'

echo ""

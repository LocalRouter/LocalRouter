#!/bin/bash
# Test script for unified MCP and OpenAI endpoints

BASE_URL="http://localhost:3625"
CLIENT_ID="test-client-123"

echo "=== Testing Unified API Endpoints ==="
echo ""

# Test 1: Root endpoint (GET - should return API docs)
echo "1. Testing GET / (API Documentation)"
curl -s "$BASE_URL/" | head -5
echo ""
echo ""

# Test 2: Health check
echo "2. Testing GET /health"
curl -s -w "\nStatus: %{http_code}\n" "$BASE_URL/health"
echo ""
echo ""

# Test 3: OpenAPI spec
echo "3. Testing GET /openapi.json"
curl -s "$BASE_URL/openapi.json" | jq -r '.openapi, .info.title, .info.version' 2>/dev/null || echo "OpenAPI endpoint exists"
echo ""
echo ""

# Test 4: List models (OpenAI compatible)
echo "4. Testing GET /models (OpenAI compatible)"
curl -s -w "\nStatus: %{http_code}\n" \
  -H "Authorization: Bearer $CLIENT_ID" \
  "$BASE_URL/models" | jq -r '.object' 2>/dev/null || echo "Response received"
echo ""
echo ""

# Test 5: List models with /v1 prefix
echo "5. Testing GET /v1/models (with prefix)"
curl -s -w "\nStatus: %{http_code}\n" \
  -H "Authorization: Bearer $CLIENT_ID" \
  "$BASE_URL/v1/models" | jq -r '.object' 2>/dev/null || echo "Response received"
echo ""
echo ""

# Test 6: MCP unified gateway (POST /)
echo "6. Testing POST / (MCP Unified Gateway)"
curl -s -w "\nStatus: %{http_code}\n" \
  -X POST \
  -H "Authorization: Bearer $CLIENT_ID" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/list",
    "params": {}
  }' \
  "$BASE_URL/"
echo ""
echo ""

# Test 7: Individual MCP server (if server exists)
echo "7. Testing POST /mcp/{server_id} (Individual Server Proxy)"
echo "Note: This will fail if no server with this ID exists, which is expected"
curl -s -w "\nStatus: %{http_code}\n" \
  -X POST \
  -H "Authorization: Bearer $CLIENT_ID" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/list",
    "params": {}
  }' \
  "$BASE_URL/mcp/test-server"
echo ""
echo ""

# Test 8: OAuth token endpoint
echo "8. Testing POST /oauth/token"
curl -s -w "\nStatus: %{http_code}\n" \
  -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "grant_type": "client_credentials",
    "client_id": "test",
    "client_secret": "test"
  }' \
  "$BASE_URL/oauth/token"
echo ""
echo ""

echo "=== Endpoint Testing Complete ==="
echo ""
echo "Summary of Unified API:"
echo "  - GET /          → API documentation"
echo "  - POST /         → MCP unified gateway"
echo "  - GET /health    → Health check"
echo "  - GET /models    → OpenAI models (with or without /v1)"
echo "  - POST /mcp/:id  → Individual MCP server proxy"
echo "  - POST /oauth/token → OAuth token endpoint"

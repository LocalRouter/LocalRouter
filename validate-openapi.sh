#!/bin/bash
# OpenAPI Specification Validation Script
#
# This script validates the OpenAPI specification using external tools.
# It performs the following checks:
# 1. Validates spec with swagger-cli
# 2. Optionally generates a TypeScript client to verify usability
# 3. Provides detailed error reporting

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
SERVER_URL="${SERVER_URL:-http://localhost:3625}"
SPEC_PATH="${SERVER_URL}/openapi.json"
VALIDATE_ONLY="${VALIDATE_ONLY:-false}"

echo "============================================"
echo "OpenAPI Specification Validation"
echo "============================================"
echo ""

# Function to print status messages
print_status() {
    echo -e "${GREEN}✓${NC} $1"
}

print_error() {
    echo -e "${RED}✗${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}⚠${NC} $1"
}

# Check if server is running
echo "Checking if server is running at ${SERVER_URL}..."
if curl -s -f "${SERVER_URL}/health" > /dev/null 2>&1; then
    print_status "Server is running"
else
    print_error "Server is not running at ${SERVER_URL}"
    echo ""
    echo "Please start the server first:"
    echo "  cargo tauri dev"
    echo ""
    echo "Or set SERVER_URL environment variable to point to a running server:"
    echo "  SERVER_URL=http://localhost:8080 $0"
    exit 1
fi

# Check if swagger-cli is installed
echo ""
echo "Checking for validation tools..."
if ! command -v npx &> /dev/null; then
    print_error "npx not found (required for swagger-cli)"
    echo ""
    echo "Please install Node.js and npm:"
    echo "  https://nodejs.org/"
    exit 1
fi

print_status "npx is available"

# Fetch the OpenAPI spec
echo ""
echo "Fetching OpenAPI specification from ${SPEC_PATH}..."
TEMP_SPEC=$(mktemp)
trap "rm -f ${TEMP_SPEC}" EXIT

if curl -s -f "${SPEC_PATH}" -o "${TEMP_SPEC}"; then
    print_status "Spec downloaded successfully"
else
    print_error "Failed to fetch OpenAPI spec from ${SPEC_PATH}"
    exit 1
fi

# Validate spec structure
echo ""
echo "Checking spec structure..."
if jq -e '.openapi' "${TEMP_SPEC}" > /dev/null 2>&1; then
    OPENAPI_VERSION=$(jq -r '.openapi' "${TEMP_SPEC}")
    print_status "OpenAPI version: ${OPENAPI_VERSION}"
else
    print_error "Invalid JSON or missing 'openapi' field"
    exit 1
fi

if jq -e '.info.title' "${TEMP_SPEC}" > /dev/null 2>&1; then
    API_TITLE=$(jq -r '.info.title' "${TEMP_SPEC}")
    print_status "API title: ${API_TITLE}"
else
    print_error "Missing 'info.title' field"
    exit 1
fi

# Count endpoints
ENDPOINT_COUNT=$(jq '.paths | length' "${TEMP_SPEC}")
print_status "Found ${ENDPOINT_COUNT} endpoints"

# Validate with swagger-cli
echo ""
echo "Validating with swagger-cli..."
if npx --yes @apidevtools/swagger-cli validate "${TEMP_SPEC}"; then
    print_status "OpenAPI spec is valid according to swagger-cli"
else
    print_error "OpenAPI spec validation failed"
    exit 1
fi

# Additional checks
echo ""
echo "Running additional checks..."

# Check for security schemes
if jq -e '.components.securitySchemes.bearer_auth' "${TEMP_SPEC}" > /dev/null 2>&1; then
    print_status "Bearer authentication scheme present"
else
    print_warning "Bearer authentication scheme missing"
fi

if jq -e '.components.securitySchemes.oauth2' "${TEMP_SPEC}" > /dev/null 2>&1; then
    print_status "OAuth2 scheme present"
else
    print_warning "OAuth2 scheme missing"
fi

# Check for common endpoints
REQUIRED_ENDPOINTS=(
    "/v1/chat/completions"
    "/v1/completions"
    "/v1/embeddings"
    "/v1/models"
    "/health"
)

for endpoint in "${REQUIRED_ENDPOINTS[@]}"; do
    if jq -e ".paths.\"${endpoint}\"" "${TEMP_SPEC}" > /dev/null 2>&1; then
        print_status "Endpoint ${endpoint} present"
    else
        print_warning "Endpoint ${endpoint} missing"
    fi
done

# Generate TypeScript client (optional)
if [ "${VALIDATE_ONLY}" != "true" ]; then
    echo ""
    echo "Generating TypeScript client to verify usability..."
    TEMP_CLIENT=$(mktemp)
    trap "rm -f ${TEMP_SPEC} ${TEMP_CLIENT}" EXIT

    if npx --yes openapi-typescript "${TEMP_SPEC}" -o "${TEMP_CLIENT}" 2>/dev/null; then
        print_status "TypeScript client generated successfully"

        # Check if types were generated
        if [ -s "${TEMP_CLIENT}" ]; then
            TYPE_COUNT=$(grep -c "export interface" "${TEMP_CLIENT}" || true)
            print_status "Generated ${TYPE_COUNT} TypeScript interfaces"
        fi
    else
        print_warning "Failed to generate TypeScript client (non-critical)"
    fi
fi

# Summary
echo ""
echo "============================================"
echo -e "${GREEN}Validation Summary${NC}"
echo "============================================"
echo ""
echo "OpenAPI Version:  ${OPENAPI_VERSION}"
echo "API Title:        ${API_TITLE}"
echo "Endpoints:        ${ENDPOINT_COUNT}"
echo "Spec URL:         ${SPEC_PATH}"
echo ""
print_status "All validation checks passed!"
echo ""
echo "To view the spec in a browser:"
echo "  curl ${SPEC_PATH} | jq ."
echo ""
echo "To download the spec:"
echo "  curl ${SPEC_PATH} > openapi.json"
echo ""

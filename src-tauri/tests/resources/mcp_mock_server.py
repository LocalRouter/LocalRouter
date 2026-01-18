#!/usr/bin/env python3
"""
MCP STDIO Mock Server for Testing

Reads JSON-RPC 2.0 requests from stdin and writes responses to stdout.
Configuration is provided via environment variables.
"""

import sys
import json
import time
import os
import threading

def load_config():
    """Load mock configuration from environment variables."""
    responses = {}
    errors = {}
    delay_seconds = 0
    hang_forever = False

    # Load responses from MCP_MOCK_RESPONSES (JSON)
    responses_json = os.environ.get('MCP_MOCK_RESPONSES', '{}')
    if responses_json:
        try:
            responses = json.loads(responses_json)
        except json.JSONDecodeError:
            print(f"ERROR: Invalid MCP_MOCK_RESPONSES JSON", file=sys.stderr, flush=True)

    # Load errors from MCP_MOCK_ERRORS (JSON)
    errors_json = os.environ.get('MCP_MOCK_ERRORS', '{}')
    if errors_json:
        try:
            errors = json.loads(errors_json)
        except json.JSONDecodeError:
            print(f"ERROR: Invalid MCP_MOCK_ERRORS JSON", file=sys.stderr, flush=True)

    # Load delay setting
    delay_str = os.environ.get('MCP_MOCK_DELAY', '0')
    try:
        delay_seconds = int(delay_str)
    except ValueError:
        print(f"ERROR: Invalid MCP_MOCK_DELAY value: {delay_str}", file=sys.stderr, flush=True)

    # Load hang forever setting
    hang_forever = os.environ.get('MCP_MOCK_HANG', '').lower() in ('1', 'true', 'yes')

    return responses, errors, delay_seconds, hang_forever

def process_request(request_line, responses, errors, delay_seconds):
    """Process a single JSON-RPC request and return response."""
    try:
        request = json.loads(request_line)
    except json.JSONDecodeError as e:
        # Parse error
        return {
            'jsonrpc': '2.0',
            'id': None,
            'error': {
                'code': -32700,
                'message': f'Parse error: {str(e)}'
            }
        }

    # Extract request details
    method = request.get('method')
    req_id = request.get('id')

    # Apply delay if configured
    if delay_seconds > 0:
        time.sleep(delay_seconds)

    # Check if this is a notification (no response needed)
    if req_id is None:
        return None

    # Check for error response
    if method in errors:
        error_data = errors[method]
        if isinstance(error_data, list) and len(error_data) >= 2:
            error_code, error_message = error_data[0], error_data[1]
        elif isinstance(error_data, dict):
            error_code = error_data.get('code', -32603)
            error_message = error_data.get('message', 'Internal error')
        else:
            error_code = -32603
            error_message = str(error_data)

        return {
            'jsonrpc': '2.0',
            'id': req_id,
            'error': {
                'code': error_code,
                'message': error_message
            }
        }

    # Check for success response
    if method in responses:
        return {
            'jsonrpc': '2.0',
            'id': req_id,
            'result': responses[method]
        }

    # Method not found
    return {
        'jsonrpc': '2.0',
        'id': req_id,
        'error': {
            'code': -32601,
            'message': f'Method not found: {method}'
        }
    }

def main():
    """Main event loop."""
    # Load configuration
    responses, errors, delay_seconds, hang_forever = load_config()

    # Debug: log configuration to stderr
    print(f"MCP Mock Server starting...", file=sys.stderr, flush=True)
    print(f"Responses: {len(responses)} methods", file=sys.stderr, flush=True)
    print(f"Errors: {len(errors)} methods", file=sys.stderr, flush=True)
    print(f"Delay: {delay_seconds}s", file=sys.stderr, flush=True)
    print(f"Hang: {hang_forever}", file=sys.stderr, flush=True)

    # If hang_forever is set, just sleep forever
    if hang_forever:
        print("HANGING FOREVER", file=sys.stderr, flush=True)
        while True:
            time.sleep(1)

    # Process requests from stdin
    try:
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue

            # Process request
            response = process_request(line, responses, errors, delay_seconds)

            # Write response if not a notification
            if response is not None:
                print(json.dumps(response), flush=True)

    except KeyboardInterrupt:
        print("Mock server interrupted", file=sys.stderr, flush=True)
    except Exception as e:
        print(f"Mock server error: {e}", file=sys.stderr, flush=True)

if __name__ == '__main__':
    main()

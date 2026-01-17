# Development Guide

## File-Based Secrets Storage (Development Mode)

By default, LocalRouter AI stores secrets (API keys, provider keys) in your system's secure keychain:
- **macOS**: Keychain
- **Windows**: Credential Manager
- **Linux**: Secret Service / keyutils

This provides maximum security but requires permission prompts every time the app accesses secrets.

### Enable File-Based Storage for Development

For development, you can use file-based storage to avoid constant permission prompts:

```bash
export LOCALROUTER_KEYCHAIN=file
```

Add this to your shell profile (`~/.zshrc`, `~/.bashrc`, etc.) to make it permanent:

```bash
# For development only - use file-based keychain storage
export LOCALROUTER_KEYCHAIN=file
```

**⚠️ WARNING**: File-based storage stores secrets in **plain text** at `~/.localrouter/secrets.json`. This is **NOT secure** and should **ONLY** be used for development with test API keys. Never use this in production or with real API keys.

### How It Works

The system uses a thin wrapper architecture:

1. **`KeychainStorage` trait**: Common interface for all storage backends
2. **`SystemKeychain`**: Uses OS keyring (secure, requires permissions)
3. **`FileKeychain`**: Uses JSON file (insecure, no permissions needed)
4. **`MockKeychain`**: In-memory only (for tests)
5. **`CachedKeychain`**: Wraps any backend with in-memory caching

When you set `LOCALROUTER_KEYCHAIN=file`, the app automatically uses `FileKeychain` instead of `SystemKeychain`, but all other logic (caching, API key management, etc.) remains the same.

### Switching Back to System Keychain

Simply unset the environment variable:

```bash
unset LOCALROUTER_KEYCHAIN
```

Or set it explicitly:

```bash
export LOCALROUTER_KEYCHAIN=system
```

### Security Notes

- `secrets.json` is already in `.gitignore` (via `*.localrouter/` pattern)
- The file contains all secrets in plain JSON format
- Anyone with file system access can read your API keys
- Use only for local development with test/disposable keys
- Production builds should always use the system keychain

### Inspecting Secrets File

If you need to debug or inspect stored secrets:

```bash
cat ~/.localrouter/secrets.json | jq .
```

The format is:

```json
{
  "service:account": "secret_value",
  "LocalRouter-APIKeys:key-id-123": "lr-abc123...",
  "LocalRouter-Providers:openai": "sk-proj-..."
}
```

### Implementation Details

The caching layer works the same regardless of backend:

1. First access: Fetch from storage (file or keychain) → cache in memory
2. Subsequent access: Return from memory cache
3. Updates: Write to storage + update cache
4. Deletes: Remove from storage + invalidate cache

This means file-based storage is just as fast as system keychain after the initial load.

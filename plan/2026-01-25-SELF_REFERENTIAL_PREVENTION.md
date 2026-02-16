# Plan: Prevent Self-Referential Provider Configuration

## Problem
Users can accidentally create a provider that points back to LocalRouter itself. This causes:
1. Infinite loop attempts when listing models
2. Parse errors when the response doesn't match expected format
3. Confusing error messages

## Solution
Detect self-referential providers by checking if the provider's `api_key` matches LocalRouter's own client API key format.

**Key insight**: LocalRouter client API keys have a unique format:
- Prefix: `lr-`
- Length: exactly 46 characters
- Format: `lr-{43 base64url chars}`

No legitimate external provider (OpenAI, Anthropic, etc.) uses this format. If a provider is configured with an `lr-` prefixed key, it must be pointing to LocalRouter.

## Files to Modify

1. `src-tauri/src/config/validation.rs` - Add API key format validation

## Implementation

### In `src-tauri/src/config/validation.rs`:

**1. Add constant for LocalRouter key format:**
```rust
/// LocalRouter client API keys start with this prefix
const LOCALROUTER_KEY_PREFIX: &str = "lr-";
/// LocalRouter client API keys are exactly this length
const LOCALROUTER_KEY_LENGTH: usize = 46;
```

**2. Add helper function:**
```rust
/// Check if an API key matches LocalRouter's client key format
fn is_localrouter_api_key(api_key: &str) -> bool {
    api_key.starts_with(LOCALROUTER_KEY_PREFIX) && api_key.len() == LOCALROUTER_KEY_LENGTH
}
```

**3. Add validation function:**
```rust
/// Validate that no provider is configured with a LocalRouter client API key
/// This prevents accidental self-referential configurations
fn validate_providers_not_self_referential(providers: &[ProviderConfig]) -> AppResult<()> {
    for provider in providers {
        // Check api_key in provider_config JSON
        let api_key = provider
            .provider_config
            .as_ref()
            .and_then(|c| c.get("api_key"))
            .and_then(|v| v.as_str());

        if let Some(key) = api_key {
            if is_localrouter_api_key(key) {
                return Err(AppError::Config(format!(
                    "Provider '{}' is configured with a LocalRouter client API key. \
                     This would create a request loop. Use an external provider's API key instead.",
                    provider.name
                )));
            }
        }
    }
    Ok(())
}
```

**4. Update `validate_config()` to call the new validation (after line 15):**
```rust
pub fn validate_config(config: &AppConfig) -> AppResult<()> {
    validate_server_config(config)?;
    validate_providers(&config.providers)?;
    validate_providers_not_self_referential(&config.providers)?;  // ADD THIS LINE
    validate_strategies(config)?;
    validate_cross_references(config)?;
    validate_client_strategy_refs(config)?;
    Ok(())
}
```

## Why This Approach

| Approach | Pros | Cons |
|----------|------|------|
| **API key detection** (chosen) | Can't be bypassed by using different hostnames; simple; no false positives | - |
| Host/port blocking | Simple | Easily bypassed with different hostname/IP |
| Request-time detection | Defense in depth | Only catches at runtime, not config time |

## Test Cases

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_localrouter_api_key() {
        // Valid LocalRouter key format
        assert!(is_localrouter_api_key("lr-8xIF-tmewuD4eOm1dxHKRjiCAD57nLAGRLEJISS1K6E"));

        // Too short
        assert!(!is_localrouter_api_key("lr-short"));

        // Wrong prefix
        assert!(!is_localrouter_api_key("sk-1234567890123456789012345678901234567890123"));

        // OpenAI key format
        assert!(!is_localrouter_api_key("sk-proj-abcdefghijklmnopqrstuvwxyz123456"));
    }

    #[test]
    fn test_self_referential_provider_blocked() {
        let mut config = AppConfig::default();
        config.providers.push(ProviderConfig {
            name: "Bad Provider".to_string(),
            provider_type: "custom".to_string(),
            enabled: true,
            provider_config: Some(serde_json::json!({
                "base_url": "http://some-host:8080",
                "api_key": "lr-8xIF-tmewuD4eOm1dxHKRjiCAD57nLAGRLEJISS1K6E"
            })),
        });

        let result = validate_config(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("LocalRouter client API key"));
    }

    #[test]
    fn test_external_provider_allowed() {
        let mut config = AppConfig::default();
        config.providers.push(ProviderConfig {
            name: "OpenAI".to_string(),
            provider_type: "openai".to_string(),
            enabled: true,
            provider_config: Some(serde_json::json!({
                "api_key": "sk-proj-abcdefghijklmnopqrstuvwxyz123456"
            })),
        });

        assert!(validate_config(&config).is_ok());
    }
}
```

## Verification

1. `cargo test config::validation` - Run validation tests
2. `cargo clippy` - Check for issues
3. `cargo tauri dev` - Start app
4. Try creating a custom provider with a LocalRouter client API key
5. Verify error message: "Provider 'X' is configured with a LocalRouter client API key..."

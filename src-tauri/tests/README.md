# Integration Tests

This directory contains comprehensive integration tests for LocalRouter AI.

## Test Categories

### 1. Mock Keychain Tests (Default)
These tests use an in-memory mock keychain and run automatically in CI/CD:
- ✅ No system keychain access required
- ✅ No user interaction needed
- ✅ Fast and deterministic
- ✅ Safe for automated testing

**Run with:**
```bash
cargo test --test integration_config_tests
```

**Coverage:**
- Configuration file loading, saving, and reloading
- API key creation, listing, verification, and deletion
- Config + API key persistence
- Multiple API keys management
- Full workflow integration

### 2. Real Keychain Tests (Manual Verification)
These tests use the actual system keychain and are ignored by default:
- ✅ Now works with `keyring` v3.6 + `apple-native` feature
- ⚠️ Requires user interaction on first run (keychain access permission)
- ⚠️ Ignored by default to avoid prompts in CI/CD
- ✅ Useful for manual verification before releases

**Run with:**
```bash
cargo test --test integration_config_tests -- --ignored --nocapture
```

**Or run specific tests:**
```bash
# Test basic keychain integration
cargo test test_real_keychain_integration -- --ignored --nocapture

# Test key rotation
cargo test test_real_keychain_rotation -- --ignored --nocapture

# Test full integration (keychain + config)
cargo test test_real_keychain_with_config_persistence -- --ignored --nocapture
```

**What to expect:**
- **macOS**: You'll see Touch ID or password prompts to access Keychain
- **Windows**: Credential Manager may prompt for access
- **Linux**: Secret Service D-Bus authentication may be required

**Coverage:**
- Real system keychain storage and retrieval
- Key creation, verification, and deletion with actual keychain
- Key rotation (security feature)
- Full integration with config file + real keychain

## Test Results

### Regular Tests (Mock Keychain)
```
running 19 tests
test result: ok. 16 passed; 0 failed; 3 ignored
```

### Real Keychain Tests
Must be run manually with `--ignored` flag.

## Why Two Test Suites?

### Mock Keychain (Default)
- **Speed**: Tests run in milliseconds
- **Reliability**: No external dependencies or user interaction
- **CI/CD**: Works in automated environments
- **Coverage**: Tests the business logic thoroughly

### Real Keychain (Manual)
- **Integration**: Verifies actual system keychain works
- **Platform-specific**: Catches platform-specific issues
- **Security**: Ensures proper keychain permissions and access
- **Production verification**: Confirms real-world usage works

## File Structure

```
tests/
├── README.md                       # This file
└── integration_config_tests.rs     # All integration tests
    ├── Mock Keychain Tests (16)    # Run by default
    └── Real Keychain Tests (3)     # Run with --ignored
```

## Adding New Tests

### For Mock Keychain Tests
Just add a normal `#[tokio::test]` function:

```rust
#[tokio::test]
async fn test_my_feature() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = ApiKeyManager::with_keychain(vec![], mock_keychain);
    // ... test code
}
```

### For Real Keychain Tests
Mark with `#[ignore]` and add user-facing messages:

```rust
#[tokio::test]
#[serial]
#[ignore]
async fn test_real_keychain_feature() {
    println!("🔐 Testing real keychain...");
    println!("⚠️  You may be prompted for authentication");

    let system_keychain = Arc::new(SystemKeychain);
    let manager = ApiKeyManager::with_keychain(vec![], system_keychain.clone());
    // ... test code

    // Always include cleanup!
    manager.delete_key(&key_id).expect("Cleanup failed");

    println!("✅ Test passed!");
}
```

## Troubleshooting

### macOS Keychain Access
**✅ Fixed!** The keychain now works properly with `keyring` v3.6 and the `apple-native` feature.

**First-time setup:**
- You may be prompted to allow keychain access on first run
- Click "Always Allow" to avoid repeated prompts

**How it works:**
- Uses macOS Security Framework directly via `apple-native` feature
- Stores entries in your login keychain under service `LocalRouter-APIKeys`
- Account names are UUIDs (e.g., `ed6842d7-91b5-4765-8e3a-f570d3965e22`)

**View entries in Keychain Access.app:**
1. Open Keychain Access
2. Search for "LocalRouter-APIKeys"
3. You'll see API key entries (tests clean up automatically)

### Linux Secret Service Not Available
If tests fail on Linux:
```bash
# Install gnome-keyring or similar
sudo apt-get install gnome-keyring

# Or use a different keyring backend
export KEYRING_BACKEND=pass
```

### Windows Credential Manager
Tests should work without additional setup on Windows.

## CI/CD Configuration

The mock keychain tests run automatically in CI. The real keychain tests are always skipped.

Add to your CI config:
```yaml
- name: Run integration tests
  run: cargo test --test integration_config_tests
  # Real keychain tests are automatically ignored
```

## Manual Verification Checklist

Before releases, manually run real keychain tests on each platform:

- [ ] macOS: `cargo test --test integration_config_tests -- --ignored --nocapture`
- [ ] Windows: `cargo test --test integration_config_tests -- --ignored --nocapture`
- [ ] Linux: `cargo test --test integration_config_tests -- --ignored --nocapture`

All tests should pass with appropriate user interaction.

//! Test keychain with external verification
//! Run with: cargo run --example test_keychain_with_verify

use localrouter_ai::api_keys::{ApiKeyManager, SystemKeychain};
use std::process::Command;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    println!("🔐 Testing with external verification\n");

    let system_keychain = Arc::new(SystemKeychain);
    let manager = ApiKeyManager::with_keychain(vec![], system_keychain.clone());

    // Create key
    let (key, config) = manager
        .create_key(Some("verify-test".to_string()))
        .await
        .expect("Failed to create key");

    println!("✅ Created key with ID: {}", config.id);
    println!("   Key value: {}...\n", &key[..20]);

    // Try to retrieve using security command
    println!("📋 Checking with macOS 'security' command...");
    let output = Command::new("security")
        .args(&[
            "find-generic-password",
            "-s",
            "LocalRouter-APIKeys",
            "-a",
            &config.id,
            "-w",
        ])
        .output()
        .expect("Failed to run security command");

    if output.status.success() {
        let retrieved = String::from_utf8_lossy(&output.stdout);
        println!("   ✅ Found in keychain: {}...", &retrieved[..20.min(retrieved.len())]);
        if retrieved.trim() == key {
            println!("   ✅ Keys match!");
        } else {
            println!("   ❌ Keys don't match!");
        }
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        println!("   ❌ Not found: {}", error.trim());
    }

    // Cleanup
    println!("\n🧹 Cleaning up...");
    manager.delete_key(&config.id).expect("Failed to delete");
    println!("   ✅ Deleted");
}

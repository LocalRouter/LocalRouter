//! Test API key creation with keychain
//! Run with: cargo run --example test_api_key_keychain

use localrouter_ai::api_keys::keychain_trait::{KeychainStorage, SystemKeychain};
use localrouter_ai::api_keys::ApiKeyManager;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    println!("🔐 Testing API Key Manager with System Keychain\n");

    let system_keychain = Arc::new(SystemKeychain);
    let manager = ApiKeyManager::with_keychain(vec![], system_keychain.clone());

    println!("1️⃣  Creating API key...");
    let result = manager
        .create_key(Some("example-test-key".to_string()))
        .await;

    match result {
        Ok((key, config)) => {
            println!("   ✅ Created key: {}", config.name);
            println!("      ID: {}", config.id);
            println!("      Key: {}...", &key[..20]);

            println!("\n2️⃣  Retrieving key from keychain...");
            match system_keychain.get("LocalRouter-APIKeys", &config.id) {
                Ok(Some(retrieved)) => {
                    println!("   ✅ Retrieved from keychain directly");
                    if retrieved == key {
                        println!("      ✅ Keys match!");
                    } else {
                        println!("      ❌ Keys don't match!");
                    }
                }
                Ok(None) => {
                    println!("   ❌ Key not found in keychain");
                }
                Err(e) => {
                    println!("   ❌ Error retrieving: {:?}", e);
                }
            }

            println!("\n3️⃣  Retrieving through manager...");
            match manager.get_key_value(&config.id) {
                Ok(Some(retrieved)) => {
                    println!("   ✅ Retrieved through manager");
                    if retrieved == key {
                        println!("      ✅ Keys match!");
                    } else {
                        println!("      ❌ Keys don't match!");
                    }
                }
                Ok(None) => {
                    println!("   ❌ Key not found through manager");
                }
                Err(e) => {
                    println!("   ❌ Error retrieving: {:?}", e);
                }
            }

            println!("\n4️⃣  Cleaning up...");
            match manager.delete_key(&config.id) {
                Ok(()) => println!("   ✅ Key deleted"),
                Err(e) => println!("   ❌ Delete failed: {:?}", e),
            }
        }
        Err(e) => {
            println!("   ❌ Failed to create key: {:?}", e);
        }
    }

    println!("\n✨ Test complete!");
}

//! Simple keychain diagnostic tool
//! Run with: cargo run --example test_keychain

use keyring::Entry;

fn main() {
    println!("🔐 Testing keyring crate with macOS Keychain\n");

    let service = "LocalRouter-DiagnosticTest";
    let account = "test-account";
    let password = "test-password-12345";

    println!("1️⃣  Creating keyring entry...");
    let entry = Entry::new(service, account).expect("Failed to create entry");

    println!("2️⃣  Setting password...");
    match entry.set_password(password) {
        Ok(()) => println!("   ✅ Password set successfully"),
        Err(e) => {
            println!("   ❌ Failed to set password: {}", e);
            return;
        }
    }

    println!("3️⃣  Getting password...");
    match entry.get_password() {
        Ok(retrieved) => {
            if retrieved == password {
                println!("   ✅ Password retrieved successfully: {}", retrieved);
            } else {
                println!("   ❌ Password mismatch!");
                println!("      Expected: {}", password);
                println!("      Got: {}", retrieved);
            }
        }
        Err(e) => {
            println!("   ❌ Failed to get password: {}", e);
            println!("      Error type: {:?}", e);
        }
    }

    println!("4️⃣  Deleting password...");
    match entry.delete_credential() {
        Ok(()) => println!("   ✅ Password deleted successfully"),
        Err(e) => println!("   ⚠️  Delete failed (might not exist): {}", e),
    }

    println!("\n✨ Diagnostic complete!");
}

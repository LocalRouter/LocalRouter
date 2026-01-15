//! Check what entries the keyring crate is actually creating
//! Run with: cargo run --example check_keychain_entries

use keyring::Entry;

fn main() {
    println!("🔍 Checking keychain entries created by keyring crate\n");

    // Create a test entry
    let test_service = "LocalRouter-APIKeys";
    let test_account = "test-check-entry";
    let test_password = "lr-TestPassword123";

    println!("1️⃣  Creating entry with:");
    println!("   Service: {}", test_service);
    println!("   Account: {}", test_account);
    println!("   Password: {}...", &test_password[..10]);

    let entry = Entry::new(test_service, test_account).expect("Failed to create entry");

    match entry.set_password(test_password) {
        Ok(()) => println!("   ✅ set_password returned Ok"),
        Err(e) => {
            println!("   ❌ set_password failed: {}", e);
            return;
        }
    }

    // Try to retrieve it
    println!("\n2️⃣  Retrieving entry...");
    match entry.get_password() {
        Ok(password) => {
            println!("   ✅ Retrieved: {}...", &password[..10]);
            if password == test_password {
                println!("   ✅ Password matches!");
            }
        }
        Err(e) => {
            println!("   ❌ Failed to retrieve: {:?}", e);
        }
    }

    // Now check with security command
    println!("\n3️⃣  Checking with macOS security command...");
    println!("   Run this command to see where it was stored:");
    println!("   security find-generic-password -a \"{}\" -w 2>&1", test_account);

    println!("\n4️⃣  Or search for all entries:");
    println!("   security dump-keychain | grep -B 2 -A 2 \"{}\"", test_account);

    // Cleanup
    println!("\n5️⃣  Cleaning up...");
    match entry.delete_credential() {
        Ok(()) => println!("   ✅ Deleted"),
        Err(e) => println!("   ⚠️  Delete returned: {:?}", e),
    }

    println!("\n💡 If the entry was found with get_password but not with 'security',");
    println!("   the keyring crate is using a fallback/temporary storage.");
}

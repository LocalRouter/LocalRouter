//! Test keychain WITHOUT deleting to see if it persists
//! Run with: cargo run --example test_keychain_no_delete

use keyring::Entry;

fn main() {
    println!("🔐 Testing keyring WITHOUT deletion\n");

    let service = "LocalRouter-PersistTest";
    let account = "test-persist";
    let password = "test-password-persist-12345";

    println!("1️⃣  Creating entry...");
    let entry = Entry::new(service, account).expect("Failed to create entry");

    println!("2️⃣  Setting password...");
    entry.set_password(password).expect("Failed to set password");
    println!("   ✅ Password set");

    println!("3️⃣  Getting password with SAME entry object...");
    let retrieved1 = entry.get_password().expect("Failed to get password");
    println!("   ✅ Retrieved: {}", retrieved1);

    println!("4️⃣  Creating NEW entry object and retrieving...");
    let entry2 = Entry::new(service, account).expect("Failed to create entry2");
    match entry2.get_password() {
        Ok(retrieved2) => println!("   ✅ Retrieved with new entry: {}", retrieved2),
        Err(e) => println!("   ❌ Failed with new entry: {:?}", e),
    }

    println!("\n5️⃣  Check with security command:");
    println!("   security find-generic-password -s \"{}\" -a \"{}\" -w", service, account);

    println!("\n⚠️  NOT deleting - check if it persists!");
}

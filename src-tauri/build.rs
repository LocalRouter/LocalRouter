// Build script for LocalRouter
//
// Catalog generation has been moved to crates/lr-catalog/build.rs

fn main() {
    // Run Tauri build
    tauri_build::build()
}

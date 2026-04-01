//! Build script to embed version information at compile time.
//!
//! - CI builds: Pass `BUILD_VERSION` env var (e.g., from Cargo.toml version)
//! - Local/debug builds: Falls back to "0.1"

fn main() {
    // CI sets BUILD_VERSION to semantic version from Cargo.toml
    // Local builds without BUILD_VERSION get "0.1"
    let version = std::env::var("BUILD_VERSION").unwrap_or_else(|_| "0.1".to_string());

    println!("cargo:rustc-env=BUILD_VERSION={}", version);

    // Re-run if BUILD_VERSION changes
    println!("cargo:rerun-if-env-changed=BUILD_VERSION");
}

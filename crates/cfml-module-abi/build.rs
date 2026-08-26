//! Emit the target triple as a compile-time constant.
//!
//! Cargo sets `TARGET` for build scripts but not for ordinary compilation, and
//! the triple is half of the compatibility token (§4.8): host and extension
//! each bake in their own, and the loader compares them. Deriving it from
//! `std::env::consts` instead would collapse glibc and musl into one string,
//! which is exactly the case the token exists to separate.
fn main() {
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=CFML_ABI_TARGET={}", target);
    println!("cargo:rerun-if-changed=build.rs");
}

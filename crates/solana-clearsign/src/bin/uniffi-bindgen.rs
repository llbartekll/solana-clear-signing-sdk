//! Bindings generator entry point (library mode).
//! `cargo run --features bindgen --bin uniffi-bindgen -- generate --library <dylib> --language swift --out-dir <dir>`

fn main() {
    uniffi::uniffi_bindgen_main()
}

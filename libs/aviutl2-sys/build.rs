//! vendor した SDK ヘッダに対し、構造体の並びを検める `layout.cpp` を組む

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let sdk = manifest_dir.join("vendor/aviutl2_sdk");
    let include = sdk.join("include");
    assert!(
        include.join("aviutl2_sdk/output2.h").is_file(),
        "SDK ヘッダが無い。`git submodule update --init` で {} を取得すること",
        sdk.display()
    );

    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let abi = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    assert!(
        arch == "x86_64" && abi == "msvc",
        "aviutl2-sys の対象は MSVC x64 のみ ({arch}, {abi})"
    );

    // ヘッダは構造体に `static constexpr` を持ち、コメントは UTF-8 で書かれている
    cc::Build::new()
        .cpp(true)
        .flag("/utf-8")
        .include(&include)
        .file(manifest_dir.join("src/layout.cpp"))
        .compile("aviutl2_layout");

    println!("cargo:rerun-if-changed=src/layout.cpp");
    println!("cargo:rerun-if-changed=vendor/aviutl2_sdk/include");
}

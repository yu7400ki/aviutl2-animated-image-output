//! vendor した libjxl を CMake で組み、構造体の並びを検める `layout.c` を足す

use std::env;
use std::path::{Path, PathBuf};

/// vendor に要るファイル。libjxl 本体と、置き換えの効かない 3 本の third_party
const VENDORED: [&str; 4] = [
    "libjxl/CMakeLists.txt",
    "libjxl/third_party/highway/CMakeLists.txt",
    "libjxl/third_party/brotli/CMakeLists.txt",
    "libjxl/third_party/skcms/skcms.h",
];

/// Release 構成の最適化
///
/// `cmake` クレートは `CMAKE_<LANG>_FLAGS_RELEASE` を自前の flag で上書きするため、
/// 置かなければ最適化なしで組まれる。
const RELEASE_FLAGS: &str = "/O2 /DNDEBUG";

/// libjxl の CMake へ渡すビルド設定
const CMAKE_DEFINES: [(&str, &str); 22] = [
    // Visual Studio 生成器が並べる構成を、実際に組む Release だけにする
    ("CMAKE_CONFIGURATION_TYPES", "Release"),
    ("CMAKE_C_FLAGS_RELEASE", RELEASE_FLAGS),
    ("CMAKE_CXX_FLAGS_RELEASE", RELEASE_FLAGS),
    // Rust の MSVC ターゲットは動的リリース CRT でリンクする。/MT を混ぜると噛み合わない
    ("CMAKE_MSVC_RUNTIME_LIBRARY", "MultiThreadedDLL"),
    ("BUILD_SHARED_LIBS", "OFF"),
    ("BUILD_TESTING", "OFF"),
    ("JPEGXL_ENABLE_TOOLS", "OFF"),
    ("JPEGXL_ENABLE_DEVTOOLS", "OFF"),
    ("JPEGXL_ENABLE_DOXYGEN", "OFF"),
    ("JPEGXL_ENABLE_MANPAGES", "OFF"),
    ("JPEGXL_ENABLE_BENCHMARK", "OFF"),
    ("JPEGXL_ENABLE_EXAMPLES", "OFF"),
    ("JPEGXL_ENABLE_JNI", "OFF"),
    ("JPEGXL_ENABLE_SJPEG", "OFF"),
    ("JPEGXL_ENABLE_OPENEXR", "OFF"),
    ("JPEGXL_ENABLE_VIEWERS", "OFF"),
    ("JPEGXL_ENABLE_PLUGINS", "OFF"),
    ("JPEGXL_ENABLE_TCMALLOC", "OFF"),
    ("JPEGXL_ENABLE_FUZZERS", "OFF"),
    // skcms を色管理に使い、lcms を要らなくする
    ("JPEGXL_ENABLE_SKCMS", "ON"),
    ("JPEGXL_ENABLE_TRANSCODE_JPEG", "OFF"),
    ("JPEGXL_ENABLE_BOXES", "OFF"),
];

/// 繋ぐ静的ライブラリ。依存する側から順に並べる
const LINKED: [&str; 6] = [
    "jxl",
    "jxl_threads",
    "jxl_cms",
    "hwy",
    "brotlienc",
    "brotlicommon",
];

fn main() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let vendor = manifest_dir.join("vendor");
    for path in VENDORED {
        assert!(
            vendor.join(path).is_file(),
            "libjxl のソースが揃っていない。スーパープロジェクトで \
             `git submodule update --init libs/jxl-sys/vendor/libjxl` を行い、続けて \
             `git -C libs/jxl-sys/vendor/libjxl submodule update --init \
             third_party/highway third_party/brotli third_party/skcms` で {} を取得すること",
            vendor.join(path).display()
        );
    }

    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let abi = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    assert!(
        arch == "x86_64" && abi == "msvc",
        "jxl-sys の対象は MSVC x64 のみ ({arch}, {abi})"
    );

    let install = build_libjxl(&vendor);

    // `jxl_export.h` などは CMake が生成するため、ヘッダはインストール先から読む
    cc::Build::new()
        .include(install.join("include"))
        .define("JXL_STATIC_DEFINE", None)
        .define("JXL_THREADS_STATIC_DEFINE", None)
        .file(manifest_dir.join("src/layout.c"))
        .compile("jxl_layout");

    println!(
        "cargo:rustc-link-search=native={}",
        install.join("lib").display()
    );
    for name in LINKED {
        println!("cargo:rustc-link-lib=static={name}");
    }

    println!("cargo:rerun-if-changed=vendor");
    println!("cargo:rerun-if-changed=src/layout.c");
}

/// libjxl と、それが取り込む highway / brotli / skcms を組む
///
/// 戻り値はインストール先。
fn build_libjxl(vendor: &Path) -> PathBuf {
    let mut config = cmake::Config::new(vendor.join("libjxl"));

    config.profile("Release");
    for (key, value) in CMAKE_DEFINES {
        config.define(key, value);
    }

    config.build()
}

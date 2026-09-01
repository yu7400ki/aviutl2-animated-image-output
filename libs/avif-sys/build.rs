//! vendor した libavif を CMake で組み、構造体の並びを検める `layout.c` を足す

use std::env;
use std::path::{Path, PathBuf};

/// submodule として置く vendor のディレクトリ名
const VENDORED: [&str; 3] = ["libavif", "aom", "libyuv"];

/// Release 構成の最適化。CMake の既定と同じ値を明示する
///
/// `cmake` クレートは `CMAKE_<LANG>_FLAGS_RELEASE` を自前の flag で上書きするため、
/// 置かなければ最適化なしで組まれる。CRT の選択は `CMAKE_<LANG>_FLAGS` 側が持つ。
const RELEASE_FLAGS: &str = "/O2 /Ob2 /DNDEBUG";

/// libavif の CMake へ渡すビルド設定
const CMAKE_DEFINES: [(&str, &str); 11] = [
    // libavif は生成される構成の先頭を aom の compile options の出所に選ぶ。Visual Studio
    // 生成器の既定は先頭が Debug で、Release を組んでも aom だけ Debug の flag になる
    ("CMAKE_CONFIGURATION_TYPES", "Release"),
    ("CMAKE_C_FLAGS_RELEASE", RELEASE_FLAGS),
    ("CMAKE_CXX_FLAGS_RELEASE", RELEASE_FLAGS),
    ("AVIF_CODEC_AOM", "LOCAL"),
    ("AVIF_LIBYUV", "LOCAL"),
    ("AVIF_CODEC_AOM_DECODE", "OFF"),
    ("CONFIG_AV1_HIGHBITDEPTH", "0"),
    ("AVIF_BUILD_APPS", "OFF"),
    ("AVIF_BUILD_TESTS", "OFF"),
    ("BUILD_SHARED_LIBS", "OFF"),
    ("CMAKE_MSVC_RUNTIME_LIBRARY", "MultiThreadedDLL"),
];

/// 同梱ヘッダの構造体の並びを伸縮させる実験機能
///
/// FFI が写したのは無効時の並びなので、CMake のキャッシュに残った値へ委ねず、
/// `layout.c` と同じ「無効」を毎回書き込む。
const EXPERIMENTAL_FEATURES: [&str; 3] = [
    "AVIF_ENABLE_EXPERIMENTAL_MINI",
    "AVIF_ENABLE_EXPERIMENTAL_SAMPLE_TRANSFORM",
    "AVIF_ENABLE_EXPERIMENTAL_EXTENDED_PIXI",
];

fn main() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let vendor = manifest_dir.join("vendor");
    for name in VENDORED {
        let dir = vendor.join(name);
        assert!(
            dir.join("CMakeLists.txt").is_file(),
            "{name} のソースが無い。`git submodule update --init --recursive` で {} を取得すること",
            dir.display()
        );
    }

    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let abi = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    assert!(
        arch == "x86_64" && abi == "msvc",
        "avif-sys の対象は MSVC x64 のみ ({arch}, {abi})"
    );

    let install = build_libavif(&vendor);

    cc::Build::new()
        .include(vendor.join("libavif/include"))
        .file(manifest_dir.join("src/layout.c"))
        .compile("avif_layout");

    println!(
        "cargo:rustc-link-search=native={}",
        install.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=avif");

    println!("cargo:rerun-if-changed=vendor");
    println!("cargo:rerun-if-changed=src/layout.c");
}

/// libavif と、それが取り込む aom / libyuv を組む
///
/// 静的リンクの libavif は依存ごと 1 本の `avif` へ束ねられるため、繋ぐ先はこれだけになる。
/// 戻り値はインストール先。
fn build_libavif(vendor: &Path) -> PathBuf {
    let mut config = cmake::Config::new(vendor.join("libavif"));

    // aom は Debug で組むと符号化が桁違いに遅く、テストが実用にならない。Rust の MSVC
    // ターゲットはどの profile でも動的リリース CRT を使うため、Release 固定でリンクは噛み合う
    config.profile("Release");

    for (key, value) in CMAKE_DEFINES {
        config.define(key, value);
    }
    for feature in EXPERIMENTAL_FEATURES {
        config.define(feature, "OFF");
    }

    // aom / libyuv の出所を submodule だけに閉じる。取得漏れはネットワークへ出ずエラーになる
    config.define("FETCHCONTENT_SOURCE_DIR_LIBAOM", vendor.join("aom"));
    config.define("FETCHCONTENT_SOURCE_DIR_LIBYUV", vendor.join("libyuv"));
    config.define("FETCHCONTENT_FULLY_DISCONNECTED", "ON");

    config.build()
}

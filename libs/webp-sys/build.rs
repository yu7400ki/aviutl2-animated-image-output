//! libwebp のうち、符号化と復号に要る翻訳単位をコンパイルする

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// コンパイルする翻訳単位を集める、libwebp 直下のディレクトリ
const SOURCE_DIRS: [&str; 5] = ["src/enc", "src/dec", "src/dsp", "src/utils", "sharpyuv"];

fn main() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let libwebp = manifest_dir.join("vendor/libwebp");
    assert!(
        libwebp.join("src/enc").is_dir(),
        "libwebp のソースが無い。`git submodule update --init` で {} を取得すること",
        libwebp.display()
    );

    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let abi = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    assert!(
        arch == "x86_64" && abi == "msvc",
        "webp-sys の対象は MSVC x64 のみ ({arch}, {abi})"
    );

    let (baseline, avx2) = collect_sources(&libwebp);

    build(&libwebp)
        .files(baseline)
        .file(manifest_dir.join("src/layout.c"))
        .compile("webp");

    // AVX2 の翻訳単位だけ命令セットを引き上げる。実行時にどれを呼ぶかは
    // libwebp が CPU を見て決めるため、他の翻訳単位は既定のままにする
    build(&libwebp)
        .flag("/arch:AVX2")
        .files(avx2)
        .compile("webp_avx2");

    println!("cargo:rerun-if-changed=vendor");
    println!("cargo:rerun-if-changed=src/layout.c");
}

/// `SOURCE_DIRS` の `.c` を、既定の命令セットで組むものと AVX2 のものへ分ける
fn collect_sources(libwebp: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut baseline = Vec::new();
    let mut avx2 = Vec::new();
    for dir in SOURCE_DIRS {
        let dir = libwebp.join(dir);
        let entries =
            std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{} を読めない: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("ディレクトリの読み出し").path();
            if path.extension() != Some(OsStr::new("c")) {
                continue;
            }
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            if stem.ends_with("_avx2") {
                avx2.push(path.clone());
            } else {
                baseline.push(path.clone());
            }
        }
    }
    baseline.sort();
    avx2.sort();
    (baseline, avx2)
}

fn build(libwebp: &Path) -> cc::Build {
    let mut build = cc::Build::new();
    build.include(libwebp);
    build.define("_CRT_SECURE_NO_WARNINGS", None);
    if env::var("PROFILE").as_deref() == Ok("release") {
        build.define("NDEBUG", None);
    }
    build
}

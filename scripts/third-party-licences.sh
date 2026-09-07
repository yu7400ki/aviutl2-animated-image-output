#!/usr/bin/env bash
# 配布物に同梱する第三者ライセンスを target/third-party/THIRD-PARTY-LICENCES.txt へ集める。
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

out_dir="target/third-party"
out="$out_dir/THIRD-PARTY-LICENCES.txt"

# 出所|ファイル。DLL に組み込まれる C ライブラリと SDK の許諾。
vendored=(
    "libwebp|libs/webp-sys/vendor/libwebp/COPYING"
    "libwebp|libs/webp-sys/vendor/libwebp/PATENTS"
    "libavif|libs/avif-sys/vendor/libavif/LICENSE"
    "aom|libs/avif-sys/vendor/aom/LICENSE"
    "aom|libs/avif-sys/vendor/aom/PATENTS"
    "aom/fastfeat|libs/avif-sys/vendor/aom/third_party/fastfeat/LICENSE"
    "aom/SVT-AV1|libs/avif-sys/vendor/aom/third_party/SVT-AV1/LICENSE.md"
    "aom/SVT-AV1|libs/avif-sys/vendor/aom/third_party/SVT-AV1/PATENTS.md"
    "aom/vector|libs/avif-sys/vendor/aom/third_party/vector/LICENSE"
    "aom/x86inc|libs/avif-sys/vendor/aom/third_party/x86inc/LICENSE"
    "libyuv|libs/avif-sys/vendor/libyuv/LICENSE"
    "libyuv|libs/avif-sys/vendor/libyuv/PATENTS"
    "libjxl|libs/jxl-sys/vendor/libjxl/LICENSE"
    "libjxl|libs/jxl-sys/vendor/libjxl/PATENTS"
    "highway|libs/jxl-sys/vendor/libjxl/third_party/highway/LICENSE"
    "highway|libs/jxl-sys/vendor/libjxl/third_party/highway/LICENSE-BSD3"
    "brotli|libs/jxl-sys/vendor/libjxl/third_party/brotli/LICENSE"
    "skcms|libs/jxl-sys/vendor/libjxl/third_party/skcms/LICENSE"
    "aviutl2_sdk|libs/aviutl2-sys/vendor/aviutl2_sdk/license.txt"
)

missing=()
for entry in "${vendored[@]}"; do
    path="${entry#*|}"
    [ -f "$path" ] || missing+=("$path")
done
if [ "${#missing[@]}" -ne 0 ]; then
    echo "vendor が未取得: ${#missing[@]} 件" >&2
    printf '  %s\n' "${missing[@]}" >&2
    exit 1
fi

mkdir -p "$out_dir"
tmp="$out.tmp"
rust="$out_dir/rust.tmp"
trap 'rm -f "$tmp" "$rust"' EXIT

rule="$(printf '=%.0s' {1..80})"
thin="$(printf -- '-%.0s' {1..80})"

cargo about generate --workspace --locked -o "$rust" about.hbs

{
    echo "AviUtl2 アニメーション画像出力 第三者ライセンス集"
    echo
    echo "$rule"
    echo "1. Rust クレート"
    echo "$rule"
    echo
    cat "$rust"
    echo
    echo "$rule"
    echo "2. C ライブラリと AviUtl2 SDK"
    echo "$rule"
    for entry in "${vendored[@]}"; do
        path="${entry#*|}"
        echo
        echo "$thin"
        echo "${entry%%|*} - $(basename "$path")"
        echo "$thin"
        echo
        cat "$path"
    done
} | tr -d '\r' >"$tmp"

mv "$tmp" "$out"
echo "$root/$out"

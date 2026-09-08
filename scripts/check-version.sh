#!/usr/bin/env bash
# 版が Cargo ワークスペースと aviutl2.toml で揃っていることを検める。
# 引数にタグ (v2.0.0 の形) を渡すと、v を剥いだ値とも突き合わせる。
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# package id (path+file://…/plugins/<形式>#<名前>@<版> または …#<版>) から plugins/* の版を集める。
cargo_versions="$(cargo metadata --format-version 1 --no-deps |
    grep -o '"path+file:///[^"]*/plugins/[^"/]*#[^"]*"' |
    sed 's/"$//; s/.*#//; s/.*@//' |
    sort -u || true)"

if [ -z "$cargo_versions" ]; then
    echo "plugins/* の版を cargo metadata から取れない" >&2
    exit 1
fi
if [ "$(printf '%s\n' "$cargo_versions" | wc -l)" -ne 1 ]; then
    echo "plugins/* の版が揃っていない:" >&2
    printf '%s\n' "$cargo_versions" | sed 's/^/  /' >&2
    exit 1
fi
cargo_version="$cargo_versions"

project_version="$(awk '
    /^[[:space:]]*\[/ { in_project = ($0 ~ /^[[:space:]]*\[project\][[:space:]]*$/); next }
    in_project && /^[[:space:]]*version[[:space:]]*=/ {
        if (match($0, /"[^"]*"/)) { print substr($0, RSTART + 1, RLENGTH - 2); exit }
    }
' aviutl2.toml)"

if [ -z "$project_version" ]; then
    echo "aviutl2.toml の [project] に version が無い" >&2
    exit 1
fi

mismatch=()
if [ "$project_version" != "$cargo_version" ]; then
    mismatch+=("aviutl2.toml の [project] version: $project_version")
fi

if [ "$#" -ge 1 ]; then
    tag="$1"
    case "$tag" in
    v*) ;;
    *)
        echo "タグは v から始まる: $tag" >&2
        exit 1
        ;;
    esac
    if [ "${tag#v}" != "$cargo_version" ]; then
        mismatch+=("タグ $tag: ${tag#v}")
    fi
fi

if [ "${#mismatch[@]}" -ne 0 ]; then
    echo "版が Cargo ワークスペースの $cargo_version と食い違う:" >&2
    printf '  %s\n' "${mismatch[@]}" >&2
    exit 1
fi

echo "$cargo_version"

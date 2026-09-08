#!/usr/bin/env bash
# release/ に出た 6 本の au2pkg.zip の中身を検める。引数は版 (2.0.0 の形)。
set -euo pipefail

if [ "$#" -ne 1 ]; then
    echo "usage: ${0##*/} <version>" >&2
    exit 1
fi
version="$1"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

base_id="aviutl2-animated-image-output"
formats=(png gif webp avif jxl)
licences="THIRD-PARTY-LICENCES.txt"

# zip 名|パッケージ id|Plugin 配下に在るべき auo2 (空白区切り)
expected=("$base_id-v$version.au2pkg.zip|$base_id|$(printf '%s_output.auo2 ' "${formats[@]}")")
for fmt in "${formats[@]}"; do
    expected+=("$base_id-$fmt-v$version.au2pkg.zip|$base_id-$fmt|${fmt}_output.auo2")
done

failures=()
fail() { failures+=("$1"); }

# 期待する 6 本だけが在ること。
found="$(cd release 2>/dev/null && ls -1 ./*.au2pkg.zip 2>/dev/null | sed 's|^\./||' | sort || true)"
want="$(printf '%s\n' "${expected[@]}" | cut -d'|' -f1 | sort)"
if [ "$found" != "$want" ]; then
    fail "release/ の zip が期待と違う:
$(diff <(printf '%s\n' "$want") <(printf '%s\n' "$found") | sed 's/^/    /')"
fi

for entry in "${expected[@]}"; do
    IFS='|' read -r name id plugins <<<"$entry"
    zip="release/$name"

    if [ ! -f "$zip" ]; then
        fail "$name: 無い"
        continue
    fi

    want_entries="$(
        {
            echo "package.ini"
            echo "$licences"
            for auo2 in $plugins; do echo "Plugin/$auo2"; done
        } | sort
    )"
    got_entries="$(unzip -Z1 "$zip" | tr -d '\r' | sort)"
    if [ "$got_entries" != "$want_entries" ]; then
        fail "$name: 収録物が期待と違う:
$(diff <(printf '%s\n' "$want_entries") <(printf '%s\n' "$got_entries") | sed 's/^/    /')"
    fi

    if grep -qxF package.ini <<<"$got_entries"; then
        ini="$(unzip -p "$zip" package.ini | tr -d '\r')"
        got_id="$(sed -n 's/^id=//p' <<<"$ini")"
        if [ "$got_id" != "$id" ]; then
            fail "$name: package.ini の id が $id でなく $got_id"
        fi
        information="$(sed -n 's/^information=//p' <<<"$ini")"
        case "$information" in
        *" v$version") ;;
        *) fail "$name: package.ini の information が v$version で終わらない: $information" ;;
        esac
    fi

    if grep -qxF "$licences" <<<"$got_entries"; then
        text="$(unzip -p "$zip" "$licences" | tr -d '\r')"
        for needle in "PATENTS" "aom - LICENSE"; do
            grep -qF "$needle" <<<"$text" ||
                fail "$name: $licences に \"$needle\" が無い"
        done
    fi
done

if [ "${#failures[@]}" -ne 0 ]; then
    printf '%s\n' "${failures[@]}" >&2
    exit 1
fi

printf '%s\n' "${expected[@]}" | cut -d'|' -f1 | sed 's/^/OK /'

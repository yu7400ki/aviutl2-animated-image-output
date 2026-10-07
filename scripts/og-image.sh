#!/usr/bin/env bash
# scripts/og-image/index.html を 1200x630 で描き、public/og.png へ書き出す。
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
chrome="${CHROME:-C:/Program Files/Google/Chrome/Application/chrome.exe}"

"$chrome" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
  --window-size=1200,630 --virtual-time-budget=2000 \
  --screenshot="$(cygpath -w "$root/public/og.png")" \
  "file:///$(cygpath -m "$root/scripts/og-image/index.html")"

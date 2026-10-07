import type React from "react";

import "./index.css";

const ORIGIN = "https://yu7400ki.me";
const TITLE = "AviUtl2 アニメーション画像出力プラグイン";
const DESCRIPTION =
  "AviUtl ExEdit2 で動画をアニメーション画像として出力できるプラグインセット。PNG（APNG）、GIF、WebP、AVIF、JXL の 5 つのフォーマットに対応。";

export default function Root({ children }: { children: React.ReactNode }) {
  const base = import.meta.env.BASE_URL.replace(/\/?$/, "/");
  const url = `${ORIGIN}${base}`;

  return (
    <html lang="ja">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>{TITLE}</title>
        <meta name="description" content={DESCRIPTION} />
        <meta property="og:type" content="website" />
        <meta property="og:url" content={url} />
        <meta property="og:title" content={TITLE} />
        <meta property="og:description" content={DESCRIPTION} />
        <meta property="og:locale" content="ja_JP" />
        <meta property="og:image" content={`${url}og.png`} />
        <meta property="og:image:width" content="1200" />
        <meta property="og:image:height" content="630" />
        <meta property="og:image:alt" content={TITLE} />
        <meta name="twitter:card" content="summary_large_image" />
      </head>
      <body>{children}</body>
    </html>
  );
}

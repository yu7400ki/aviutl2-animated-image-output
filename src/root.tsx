import type React from "react";

import "./index.css";

export default function Root({ children }: { children: React.ReactNode }) {
  return (
    <html lang="ja">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>AviUtl2 アニメーション画像出力プラグイン</title>
        <meta
          name="description"
          content="AviUtl ExEdit2 で動画をアニメーション画像として出力できるプラグインセット。PNG(APNG)、GIF、WebP、AVIF、JPEG XLの5つのフォーマットに対応。"
        />
      </head>
      <body>{children}</body>
    </html>
  );
}

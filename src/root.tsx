import "./index.css";
import json from "public/release.json" with { type: "json" };
import { DistributionSite } from "./components/distribution-site";
import type { PluginRelease } from "./types";

export async function getStaticPaths() {
  return ["/"];
}

export async function Root(_: { url: URL }) {
  const releases = json satisfies PluginRelease;

  return (
    <html lang="ja">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>AviUtl2 アニメーション画像出力プラグイン</title>
        <meta
          name="description"
          content="AviUtl ExEdit2 で動画をアニメーション画像として出力できるプラグインセット。PNG(APNG)、GIF、WebP、AVIFの4つのフォーマットに対応。"
        />
      </head>
      <body>
        <main>
          <DistributionSite releases={releases} />
        </main>
      </body>
    </html>
  );
}

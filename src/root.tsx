import { DistributionSite } from "./components/distribution-site";
import { getConfig, getPluginReleases } from "./libs/release";
import "./index.css";

export async function getStaticPaths() {
  return ["/"];
}

export async function Root(_: { url: URL }) {
  const config = getConfig();
  const releases = await getPluginReleases(config);

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

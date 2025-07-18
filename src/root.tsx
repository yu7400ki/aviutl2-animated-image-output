import { Counter } from "./counter";
import "./index.css";

export async function getStaticPaths() {
  return ["/"];
}

export async function Root(_: { url: URL }) {
  return (
    <html lang="ja">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>AviUtl2 アニメーション画像出力</title>
      </head>
      <body>
        <Counter />
      </body>
    </html>
  );
}

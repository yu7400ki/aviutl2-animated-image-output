import { clsx } from "clsx";

import type { Plugin } from "../libs/types";

interface PluginCardProps {
  plugin: Plugin;
  url: string | undefined;
}

const pluginInfo: Record<
  Plugin,
  {
    title: string;
    description: string;
    features: string[];
    color: string;
    textColor: string;
  }
> = {
  png: {
    title: "PNG（APNG）",
    description: "高品質、可逆圧縮",
    features: ["高品質なアニメーション画像", "可逆圧縮", "透明度対応"],
    color: "bg-green-100 border-green-300",
    textColor: "text-green-800",
  },
  gif: {
    title: "GIF",
    description: "広く対応、256 色制限、半透明は残らない",
    features: [
      "広い互換性",
      "軽量なアニメーション",
      "全フレームを通した色の和集合が 256 色以内の素材は無劣化",
      "半透明は残らない",
    ],
    color: "bg-blue-100 border-blue-300",
    textColor: "text-blue-800",
  },
  webp: {
    title: "WebP",
    description: "高圧縮率、可逆・非可逆両対応",
    features: ["高圧縮率", "ウェブ用最適化", "可逆・非可逆両対応"],
    color: "bg-purple-100 border-purple-300",
    textColor: "text-purple-800",
  },
  avif: {
    title: "AVIF",
    description: "最高の圧縮率、最新フォーマット",
    features: ["最高の圧縮率", "最新フォーマット", "最小ファイルサイズ"],
    color: "bg-orange-100 border-orange-300",
    textColor: "text-orange-800",
  },
  jxl: {
    title: "JXL",
    description: "高圧縮率、可逆・非可逆両対応、半透明が残る",
    features: [
      "高圧縮率",
      "可逆・非可逆両対応",
      "品質 100 なら入力と同じ画素",
      "画質を保ったまま配布したい場合",
    ],
    color: "bg-rose-100 border-rose-300",
    textColor: "text-rose-800",
  },
};

export function PluginCard({ plugin, url }: PluginCardProps) {
  const info = pluginInfo[plugin];

  return (
    <article className={clsx("rounded-lg border-2 p-6", info.color)}>
      <h3 className={clsx("text-xl font-bold mb-2", info.textColor)}>{info.title}</h3>
      <p className={clsx("mb-4", info.textColor)}>{info.description}</p>

      <div className="mb-4">
        <h4 className={clsx("font-semibold mb-2", info.textColor)}>特徴</h4>
        <ul
          className={clsx(
            "text-sm space-y-1 list-disc list-inside marker:text-current",
            info.textColor,
          )}
        >
          {info.features.map((feature, index) => (
            // oxlint-disable-next-line react/no-array-index-key -- 静的な一覧なので添字を key にする
            <li key={index}>{feature}</li>
          ))}
        </ul>
      </div>
      <div className="border-t border-current opacity-20 my-4" />
      {url ? (
        <a
          href={url}
          className={clsx(
            "block w-full text-center py-2 px-4 rounded font-semibold bg-white hover:bg-opacity-80 transition-colors",
            info.textColor,
          )}
          download
          aria-label={`${info.title} のパッケージをダウンロード`}
        >
          ダウンロード
        </a>
      ) : (
        <div
          className={clsx(
            "block w-full text-center py-2 px-4 rounded font-semibold bg-white opacity-50 cursor-not-allowed",
            info.textColor,
          )}
        >
          準備中
        </div>
      )}
    </article>
  );
}

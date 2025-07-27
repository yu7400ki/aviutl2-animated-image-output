import { clsx } from "clsx";
import type { Plugin, PluginRelease } from "../libs/types";

interface PluginCardProps {
  plugin: Plugin;
  release: PluginRelease;
}

const pluginInfo = {
  apng: {
    title: "PNG (APNG)",
    description: "高品質、可逆圧縮",
    features: ["高品質なアニメーション画像", "可逆圧縮", "透明度対応"],
    color: "bg-green-100 border-green-300",
    textColor: "text-green-800",
  },
  gif: {
    title: "GIF",
    description: "広く対応、256 色制限",
    features: ["広い互換性", "軽量なアニメーション", "256色制限"],
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
};

export function PluginCard({ plugin, release }: PluginCardProps) {
  const info = pluginInfo[plugin];
  const pluginRelease = release[plugin];

  return (
    <article className={clsx("rounded-lg border-2 p-6", info.color)}>
      <h3 className={clsx("text-xl font-bold mb-2", info.textColor)}>
        {info.title}
      </h3>
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
            // biome-ignore lint/suspicious/noArrayIndexKey: Using index as key for static content
            <li key={index}>{feature}</li>
          ))}
        </ul>
      </div>
      <div className="border-t border-current opacity-20 my-4" />
      {pluginRelease ? (
        <div className="space-y-2">
          <div className="flex justify-between items-center">
            <span className={clsx("font-semibold", info.textColor)}>
              バージョン: {pluginRelease.version}
            </span>
            <span className={clsx("text-sm opacity-75", info.textColor)}>
              {new Date(pluginRelease.date).toLocaleDateString("ja-JP", {
                year: "numeric",
                month: "2-digit",
                day: "2-digit",
              })}
            </span>
          </div>
          <a
            href={pluginRelease.url}
            className={clsx(
              "block w-full text-center py-2 px-4 rounded font-semibold bg-white hover:bg-opacity-80 transition-colors",
              info.textColor,
            )}
            download
            aria-label={`${info.title} バージョン ${pluginRelease.version} をダウンロード`}
          >
            ダウンロード
          </a>
        </div>
      ) : (
        <div className="space-y-2">
          <div className="flex justify-between items-center">
            <span className={clsx("font-semibold opacity-50", info.textColor)}>
              準備中
            </span>
            <span className={clsx("text-sm opacity-50", info.textColor)}>
              近日公開
            </span>
          </div>
          <div
            className={clsx(
              "block w-full text-center py-2 px-4 rounded font-semibold bg-white opacity-50 cursor-not-allowed",
              info.textColor,
            )}
          >
            準備中
          </div>
        </div>
      )}
    </article>
  );
}

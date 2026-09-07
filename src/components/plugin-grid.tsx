import { PLUGINS, type Release } from "../libs/types";
import { PluginCard } from "./plugin-card";

interface PluginGridProps {
  release: Release | undefined;
}

export function PluginGrid({ release }: PluginGridProps) {
  return (
    <section className="w-[100cqw] mx-[calc(50%-50cqw)]">
      <div className="px-4">
        <h2 className="text-2xl font-bold text-gray-900 mb-8 text-center">
          対応フォーマット
        </h2>
        <div className="max-w-2xl mx-auto mb-8 rounded-lg border-2 border-gray-300 bg-gray-50 p-6">
          <div className="flex justify-between items-center mb-2">
            <h3 className="text-lg font-bold text-gray-900">
              全部入りパッケージ
            </h3>
            {release ? (
              <span className="text-sm text-gray-600">
                バージョン {release.version} /{" "}
                {new Date(release.date).toLocaleDateString("ja-JP", {
                  year: "numeric",
                  month: "2-digit",
                  day: "2-digit",
                })}
              </span>
            ) : (
              <span className="text-sm text-gray-500">近日公開</span>
            )}
          </div>
          <p className="text-sm text-gray-600 mb-4">
            5 形式をまとめて入れる zip です。形式ごとの zip
            と混ぜずに、どちらか一方を入れてください
          </p>
          {release ? (
            <a
              href={release.bundle}
              className="block w-full text-center py-2 px-4 rounded font-semibold bg-white text-gray-900 hover:bg-gray-100 transition-colors"
              download
              aria-label={`全部入りパッケージ バージョン ${release.version} をダウンロード`}
            >
              ダウンロード
            </a>
          ) : (
            <div className="block w-full text-center py-2 px-4 rounded font-semibold bg-white text-gray-900 opacity-50 cursor-not-allowed">
              準備中
            </div>
          )}
        </div>
        <div className="grid gap-6 grid-cols-[repeat(auto-fit,minmax(18rem,1fr))]">
          {PLUGINS.map((plugin) => (
            <PluginCard
              key={plugin}
              plugin={plugin}
              url={release?.assets[plugin]}
            />
          ))}
        </div>
      </div>
    </section>
  );
}

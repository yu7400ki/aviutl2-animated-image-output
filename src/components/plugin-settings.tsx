import { clsx } from "clsx";

const pluginSettings = {
  png: {
    title: "PNG（APNG）出力設定",
    color: "green",
    items: [
      {
        name: "ループ回数",
        description: "アニメーションの繰り返し回数（0 = 無限ループ）",
      },
      {
        name: "カラーフォーマット",
        description: "透過無し / 透過付き",
      },
      {
        name: "圧縮",
        description: "標準 / 高速 / 最高",
      },
      {
        name: "アダプティブフィルター",
        description: "自動的に最適なフィルターを選択（処理時間は長くなります）",
      },
      {
        name: "フィルター",
        description: "PNG のフィルター設定（なし、Sub、Up、Average、Paeth）",
      },
    ],
  },
  gif: {
    title: "GIF 出力設定",
    color: "blue",
    items: [
      {
        name: "ループ回数",
        description: "アニメーションの繰り返し回数（0 = 無限ループ）",
      },
      {
        name: "カラーフォーマット",
        description: "透過無し / 透過付き",
      },
      {
        name: "エンコード速度",
        description: "エンコード速度（1-30、値が大きいほど高速）",
      },
    ],
  },
  webp: {
    title: "WebP 出力設定",
    color: "purple",
    items: [
      {
        name: "ループ回数",
        description: "アニメーションの繰り返し回数（0 = 無限ループ）",
      },
      {
        name: "カラーフォーマット",
        description: "透過無し / 透過付き",
      },
      {
        name: "ロスレス圧縮",
        description: "可逆圧縮の ON/OFF",
      },
      {
        name: "品質",
        description: "品質設定（0-100）",
      },
      {
        name: "メソッド",
        description: "品質と速度のトレードオフ（0-6、値が小さいほど高速）",
      },
    ],
  },
  avif: {
    title: "AVIF 出力設定",
    color: "orange",
    items: [
      {
        name: "ループ回数",
        description: "アニメーションの繰り返し回数（0 = 無限ループ）",
      },
      {
        name: "品質",
        description: "画質設定（0-100）",
      },
      {
        name: "エンコード速度",
        description: "エンコード速度（0-10、値が大きいほど高速）",
      },
      {
        name: "カラーフォーマット",
        description: "透過無し / 透過付き",
      },
      {
        name: "YUVフォーマット",
        description: "色空間設定（YUV420 / YUV422 / YUV444）",
      },
    ],
  },
};

const colorMap = {
  green: "marker:text-green-500",
  blue: "marker:text-blue-500",
  purple: "marker:text-purple-500",
  orange: "marker:text-orange-500",
} as const;

export function PluginSettings() {
  return (
    <section>
      <h2 className="text-2xl font-bold text-gray-900 mb-8">設定項目</h2>
      <p className="text-gray-600 mb-8">
        各プラグインには以下の設定項目があります：
      </p>

      <div className="space-y-8">
        {Object.entries(pluginSettings).map(([key, setting]) => (
          <div key={key}>
            <h3 className="text-lg font-semibold text-gray-900 mb-4">
              {setting.title}
            </h3>
            <ul
              className={clsx(
                "space-y-3 list-disc pl-4",
                colorMap[setting.color as keyof typeof colorMap],
              )}
            >
              {setting.items.map((item, index) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: Using index as key for static content
                <li key={index} className="space-y-1">
                  <div className="font-medium text-gray-900">{item.name}</div>
                  <div className="text-sm text-gray-600">
                    {item.description}
                  </div>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>
    </section>
  );
}

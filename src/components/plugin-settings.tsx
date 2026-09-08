import { clsx } from "clsx";

import type { Plugin } from "../libs/types";

const colorMap = {
  green: "marker:text-green-500",
  blue: "marker:text-blue-500",
  purple: "marker:text-purple-500",
  orange: "marker:text-orange-500",
  rose: "marker:text-rose-500",
} as const;

const pluginSettings: Record<
  Plugin,
  {
    title: string;
    color: keyof typeof colorMap;
    items: { name: string; description: string }[];
  }
> = {
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
        name: "圧縮レベル",
        description: "圧縮率と速度のトレードオフ（1-9、既定 6、値が大きいほど高圧縮）",
      },
      {
        name: "スレッド数",
        description: "並列にエンコードする数（1 から論理 CPU 数まで、既定は論理 CPU 数の半分）",
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
        description: "画質（0-100、既定 75）。ロスレス圧縮時はファイルサイズとのトレードオフ",
      },
      {
        name: "メソッド",
        description: "圧縮率と速度のトレードオフ（0-6、既定 4、値が小さいほど高速）",
      },
      {
        name: "スレッド数",
        description: "並列にエンコードする数（1 から論理 CPU 数まで、既定は論理 CPU 数の半分）",
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
      {
        name: "スレッド数",
        description: "並列にエンコードする数（1 から論理 CPU 数まで、既定は論理 CPU 数の半分）",
      },
    ],
  },
  jxl: {
    title: "JXL 出力設定",
    color: "rose",
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
        name: "品質",
        description:
          "画質（0-100、既定 90、100 で入力と同じ画素になる。透過付きの α は品質によらず可逆）",
      },
      {
        name: "均衡",
        description: "圧縮率と速度のトレードオフ（1-10、既定 7、値が大きいほど時間がかかる）",
      },
      {
        name: "スレッド数",
        description: "並列にエンコードする数（1 から論理 CPU 数まで、既定は論理 CPU 数の半分）",
      },
    ],
  },
};

export function PluginSettings() {
  return (
    <section>
      <h2 className="text-2xl font-bold text-gray-900 mb-8">設定項目</h2>
      <p className="text-gray-600 mb-8">各プラグインには以下の設定項目があります：</p>

      <div className="space-y-8">
        {Object.entries(pluginSettings).map(([key, setting]) => (
          <div key={key}>
            <h3 className="text-lg font-semibold text-gray-900 mb-4">{setting.title}</h3>
            <ul className={clsx("space-y-3 list-disc pl-4", colorMap[setting.color])}>
              {setting.items.map((item, index) => (
                // oxlint-disable-next-line react/no-array-index-key -- 静的な一覧なので添字を key にする
                <li key={index} className="space-y-1">
                  <div className="font-medium text-gray-900">{item.name}</div>
                  <div className="text-sm text-gray-600">{item.description}</div>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>
    </section>
  );
}

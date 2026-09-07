import { Alert } from "./ui/alert";

const notices = [
  {
    type: "warning" as const,
    title: "処理時間について",
    description: "圧縮設定や動画サイズによっては処理時間が極端に長くなる場合があります",
  },
  {
    type: "warning" as const,
    title: "ファイルサイズについて",
    description: "動画に比べてファイルサイズが大きくなる傾向があります",
  },
  {
    type: "warning" as const,
    title: "パッケージの混在について",
    description:
      "全部入りと形式ごとのパッケージは同じプラグインファイルを置きます。両方を入れた状態で片方をアンインストールすると、もう片方が使うファイルも消えます。どちらか一方だけを入れてください",
  },
];

export function Notices() {
  return (
    <section>
      <h2 className="text-2xl font-bold text-gray-900 mb-8">注意事項</h2>
      <div className="space-y-4">
        {notices.map((notice, index) => (
          <Alert
            // oxlint-disable-next-line react/no-array-index-key -- 静的な一覧なので添字を key にする
            key={index}
            type={notice.type}
            title={notice.title}
            description={notice.description}
          />
        ))}
      </div>
    </section>
  );
}

import { Alert } from "./ui/alert";

const notices = [
  {
    type: "warning" as const,
    title: "処理時間について",
    description:
      "圧縮設定や動画サイズによっては処理時間が極端に長くなる場合があります",
  },
  {
    type: "warning" as const,
    title: "ファイルサイズについて",
    description: "動画に比べてファイルサイズが大きくなる傾向があります",
  },
];

export function Notices() {
  return (
    <section>
      <h2 className="text-2xl font-bold text-gray-900 mb-8">注意事項</h2>
      <div className="space-y-4">
        {notices.map((notice, index) => (
          <Alert
            // biome-ignore lint/suspicious/noArrayIndexKey: Using index as key for static content
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

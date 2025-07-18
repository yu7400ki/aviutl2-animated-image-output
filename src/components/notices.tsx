import { Alert } from "./ui/alert";

const notices = [
  {
    type: "warning" as const,
    title: "処理時間について",
    description:
      "大きなファイルサイズや長時間の動画では処理時間が長くなる場合があります",
  },
  {
    type: "error" as const,
    title: "アルファチャンネルについて",
    description: "AviUtl2の制限でアルファチャンネルを持つ画像は出力できません",
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

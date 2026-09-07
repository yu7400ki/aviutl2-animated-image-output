import { Step } from "./ui/step";

const installationSteps = [
  {
    id: 1,
    title: "パッケージをダウンロード",
    description:
      "上のダウンロードボタンから、全部入りか使いたい形式の zip を取得してください",
  },
  {
    id: 2,
    title: "プレビュー画面にドラッグ&ドロップ",
    description:
      "ダウンロードした zip を AviUtl2 のプレビュー画面にドラッグ&ドロップするとインストールされます",
  },
  {
    id: 3,
    title: "AviUtl2 を再起動",
    description: "プラグインを認識させるため、AviUtl2を再起動してください",
  },
];

export function InstallationSteps() {
  return (
    <section>
      <h2 className="text-2xl font-bold text-gray-900 mb-8">
        インストール方法
      </h2>
      <ol className="space-y-6">
        {installationSteps.map((step) => (
          <Step
            key={step.id}
            number={step.id}
            title={step.title}
            description={step.description}
          />
        ))}
      </ol>
      <p className="text-gray-600 text-sm mt-6">
        インストールしたパッケージは、AviUtl2
        のパッケージ情報からアンインストールできます。
      </p>
    </section>
  );
}

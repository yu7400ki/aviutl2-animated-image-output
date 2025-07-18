import { Step } from "./ui/step";

const installationSteps = [
  {
    id: 1,
    title: "プラグインファイルをダウンロード",
    description:
      "上記のダウンロードボタンから最新版のプラグインファイルを取得してください",
  },
  {
    id: 2,
    title: "プラグインフォルダにコピー",
    description:
      "ダウンロードした auo2 ファイルを以下のフォルダにコピーしてください",
    code: "%ProgramData%\\aviutl2\\Plugin",
    example: "例: C:\\ProgramData\\aviutl2\\Plugin\\apng_output.auo2",
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
          >
            {step.code && (
              <code className="block bg-gray-100 px-3 py-2 rounded text-sm font-mono mt-2">
                {step.code}
              </code>
            )}
            {step.example && (
              <p className="text-gray-500 text-sm mt-2">{step.example}</p>
            )}
          </Step>
        ))}
      </ol>
    </section>
  );
}

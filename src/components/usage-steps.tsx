import { Step } from "./ui/step";

const usageSteps = [
  {
    id: 1,
    title: "プロジェクトを開く",
    description: "AviUtl2でプロジェクトを開く",
  },
  {
    id: 2,
    title: "ファイル出力を選択",
    description: "メニューから「ファイル」→「ファイル出力」を選択",
  },
  {
    id: 3,
    title: "フォーマットを選択",
    description: "出力したいフォーマットを選択",
  },
  {
    id: 4,
    title: "設定を調整",
    description: "設定ダイアログで出力オプションを調整",
  },
  {
    id: 5,
    title: "保存",
    description: "出力ファイル名を指定して保存",
  },
];

export function UsageSteps() {
  return (
    <section>
      <h2 className="text-2xl font-bold text-gray-900 mb-8">使い方</h2>
      <ol className="space-y-6">
        {usageSteps.map((step) => (
          <Step
            key={step.id}
            number={step.id}
            title={step.title}
            description={step.description}
          />
        ))}
      </ol>
    </section>
  );
}

import { Alert } from "./ui/alert";

export function Requirements() {
  return (
    <section>
      <h2 className="text-2xl font-bold text-gray-900 mb-8">動作環境</h2>
      <Alert
        type="info"
        title="必要環境"
        description="AviUtl ExEdit2 beta3 以降"
      />
    </section>
  );
}

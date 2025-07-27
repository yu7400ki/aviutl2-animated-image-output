import type { Plugin, PluginRelease } from "../libs/types";
import { PluginCard } from "./plugin-card";

interface PluginGridProps {
  releases: PluginRelease;
}

export function PluginGrid({ releases }: PluginGridProps) {
  const plugins: Plugin[] = ["apng", "gif", "webp", "avif"];

  return (
    <section className="w-[100cqw] mx-[calc(50%-50cqw)]">
      <div className="px-4">
        <h2 className="text-2xl font-bold text-gray-900 mb-8 text-center">
          対応フォーマット
        </h2>
        <div className="grid gap-6 grid-cols-[repeat(auto-fit,minmax(18rem,1fr))]">
          {plugins.map((plugin) => (
            <PluginCard key={plugin} plugin={plugin} release={releases} />
          ))}
        </div>
      </div>
    </section>
  );
}

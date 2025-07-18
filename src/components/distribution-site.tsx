import type { PluginRelease } from "../types";
import { Disclaimer } from "./disclaimer";
import { Footer } from "./footer";
import { InstallationSteps } from "./installation-steps";
import { Notices } from "./notices";
import { PluginGrid } from "./plugin-grid";
import { PluginSettings } from "./plugin-settings";
import { Requirements } from "./requirements";
import { UsageSteps } from "./usage-steps";

interface DistributionSiteProps {
  releases: PluginRelease;
}

export function DistributionSite({ releases }: DistributionSiteProps) {
  return (
    <>
      <article className="min-h-screen bg-white @container">
        <div className="max-w-4xl mx-auto px-4 py-12 space-y-16">
          <header className="text-center">
            <h1 className="text-4xl font-bold text-gray-900 mb-4">
              AviUtl2 アニメーション画像出力プラグイン
            </h1>
            <p className="text-lg text-gray-600 max-w-2xl mx-auto leading-relaxed">
              AviUtl
              ExEdit2で動画をアニメーション画像として出力するプラグインセット
            </p>
          </header>
          <PluginGrid releases={releases} />
          <Requirements />
          <InstallationSteps />
          <UsageSteps />
          <PluginSettings />
          <Notices />
          <Disclaimer />
        </div>
      </article>
      <Footer />
    </>
  );
}

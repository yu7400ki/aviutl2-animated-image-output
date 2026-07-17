import { DistributionSite } from "./components/distribution-site";
import { getConfig, getPluginReleases } from "./libs/release";

export default async function App() {
  const config = getConfig();
  const releases = await getPluginReleases(config);

  return (
    <main>
      <DistributionSite releases={releases} />
    </main>
  );
}

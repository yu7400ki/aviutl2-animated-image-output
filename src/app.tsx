import { DistributionSite } from "./components/distribution-site";
import { getConfig, getRelease } from "./libs/release";

export default async function App() {
  const config = getConfig();
  const release = await getRelease(config);

  return (
    <main>
      <DistributionSite release={release} />
    </main>
  );
}

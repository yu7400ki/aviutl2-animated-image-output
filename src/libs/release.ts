import { Octokit } from "@octokit/rest";
import type { Plugin, PluginRelease } from "./types";

interface Config {
  owner: string;
  repo: string;
}

interface ReleaseAsset {
  id: number;
  name: string;
}

interface ReleaseData {
  tag_name: string;
  published_at: string | null;
  draft: boolean;
  prerelease: boolean;
  assets: ReleaseAsset[];
}

const PLUGIN_MAP = {
  apng: {
    tagPrefix: "apng-v",
    fileName: "apng_output.auo2",
  },
  gif: {
    tagPrefix: "gif-v",
    fileName: "gif_output.auo2",
  },
  webp: {
    tagPrefix: "webp-v",
    fileName: "webp_output.auo2",
  },
  avif: {
    tagPrefix: "avif-v",
    fileName: "avif_output.auo2",
  },
} as const;

const DEFAULT_CONFIG: Config = {
  owner: "yu7400ki",
  repo: "aviutl2-animated-image-output",
};

const octokit = new Octokit();

export function getConfig(): Config {
  const owner = process.env.GITHUB_REPOSITORY_OWNER ?? DEFAULT_CONFIG.owner;
  const repo = process.env.GITHUB_REPOSITORY_NAME ?? DEFAULT_CONFIG.repo;

  return { owner, repo };
}

async function fetchReleases(config: Config): Promise<ReleaseData[]> {
  const { data: releases } = await octokit.rest.repos.listReleases({
    owner: config.owner,
    repo: config.repo,
    per_page: 100,
  });

  return releases.filter((release) => !release.draft && !release.prerelease);
}

async function downloadAsset(
  config: Config,
  asset: ReleaseAsset,
): Promise<string> {
  const response = await octokit.rest.repos.getReleaseAsset({
    owner: config.owner,
    repo: config.repo,
    asset_id: asset.id,
  });

  return response.data.browser_download_url;
}

export async function getPluginReleases(
  config: Config,
): Promise<PluginRelease> {
  const releases = await fetchReleases(config);

  const pluginRelease: PluginRelease = {};

  for (const [pluginType, pluginConfig] of Object.entries(PLUGIN_MAP)) {
    if (pluginRelease[pluginType as Plugin]) continue;

    for (const release of releases) {
      if (!release.published_at) continue;
      if (!release.tag_name.startsWith(pluginConfig.tagPrefix)) continue;

      const asset = release.assets.find(
        (a) => a.name === pluginConfig.fileName,
      );
      if (!asset) continue;

      const url = await downloadAsset(config, asset);

      pluginRelease[pluginType as Plugin] = {
        version: release.tag_name.slice(pluginConfig.tagPrefix.length),
        date: new Date(release.published_at).toISOString(),
        url,
      };
      break;
    }
  }

  return pluginRelease;
}

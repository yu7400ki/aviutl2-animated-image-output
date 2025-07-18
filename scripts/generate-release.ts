import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { Octokit } from "@octokit/rest";
import type { Plugin, PluginRelease } from "../src/types.js";

interface Config {
  token: string;
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
  png: {
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

function getConfig(): Config {
  const token = process.env.GITHUB_TOKEN;
  const owner = process.env.GITHUB_REPOSITORY_OWNER;
  const repo = process.env.GITHUB_REPOSITORY_NAME;

  if (!token || !owner || !repo) {
    console.error(
      "Missing required environment variables: GITHUB_TOKEN, GITHUB_REPOSITORY_OWNER, GITHUB_REPOSITORY_NAME",
    );
    process.exit(1);
  }

  return { token, owner, repo };
}

function createOctokit(token: string): Octokit {
  return new Octokit({ auth: token });
}

async function fetchReleases(
  octokit: Octokit,
  config: Config,
): Promise<ReleaseData[]> {
  const { data: releases } = await octokit.rest.repos.listReleases({
    owner: config.owner,
    repo: config.repo,
    per_page: 100,
  });

  return releases.filter((release) => !release.draft && !release.prerelease);
}

async function downloadAsset(
  octokit: Octokit,
  config: Config,
  assetId: number,
  fileName: string,
  publicDir: string,
): Promise<void> {
  const response = await octokit.rest.repos.getReleaseAsset({
    owner: config.owner,
    repo: config.repo,
    asset_id: assetId,
    headers: {
      Accept: "application/octet-stream",
    },
  });

  const assetPath = join(publicDir, fileName);
  writeFileSync(
    assetPath,
    Buffer.from(response.data as unknown as ArrayBuffer),
  );
}

async function processReleases(
  octokit: Octokit,
  config: Config,
  releases: ReleaseData[],
  publicDir: string,
): Promise<PluginRelease> {
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

      await downloadAsset(octokit, config, asset.id, asset.name, publicDir);

      pluginRelease[pluginType as Plugin] = {
        version: release.tag_name.slice(pluginConfig.tagPrefix.length),
        date: release.published_at,
        url: `/${asset.name}`,
      };
      break;
    }
  }

  return pluginRelease;
}

function saveReleaseJson(
  pluginRelease: PluginRelease,
  publicDir: string,
): void {
  const outputPath = join(publicDir, "release.json");
  writeFileSync(outputPath, JSON.stringify(pluginRelease, null, 2));
}

function logResults(pluginRelease: PluginRelease): void {
  console.log("✓ Generated release.json successfully");
  console.log(`  Plugins found: ${Object.keys(pluginRelease).join(", ")}`);

  for (const [plugin, info] of Object.entries(pluginRelease)) {
    console.log(`    ${plugin}: ${info.version} (${info.date})`);
  }
}

async function generateReleaseJson(): Promise<void> {
  try {
    const config = getConfig();
    const octokit = createOctokit(config.token);
    const publicDir = join(process.cwd(), "public");

    const releases = await fetchReleases(octokit, config);
    const pluginRelease = await processReleases(
      octokit,
      config,
      releases,
      publicDir,
    );

    saveReleaseJson(pluginRelease, publicDir);
    logResults(pluginRelease);
  } catch (error) {
    console.error("Error generating release.json:", error);
    process.exit(1);
  }
}

generateReleaseJson();

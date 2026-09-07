import { Octokit } from "@octokit/rest";

import type { Plugin, Release } from "./types";

interface Config {
  owner: string;
  repo: string;
}

interface ReleaseAsset {
  name: string;
  browser_download_url: string;
}

export interface ReleaseData {
  tag_name: string;
  published_at: string | null;
  draft: boolean;
  prerelease: boolean;
  assets: ReleaseAsset[];
}

type PublishedRelease = ReleaseData & { published_at: string };

const PACKAGE_ID = "aviutl2-animated-image-output";
const TAG_PREFIX = "v";

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

function isPublished(release: ReleaseData): release is PublishedRelease {
  return !release.draft && !release.prerelease && typeof release.published_at === "string";
}

function assetName(version: string, plugin?: Plugin): string {
  const format = plugin ? `-${plugin}` : "";

  return `${PACKAGE_ID}${format}-v${version}.au2pkg.zip`;
}

function assetUrl(release: ReleaseData, name: string): string | undefined {
  return release.assets.find((asset) => asset.name === name)?.browser_download_url;
}

/** 版タグの最新の公開リリースから 6 本揃ったパッケージを引く。 */
export function selectRelease(releases: ReleaseData[]): Release | undefined {
  const [latest] = releases
    .filter(isPublished)
    .filter((release) => release.tag_name.startsWith(TAG_PREFIX))
    .toSorted((a, b) => Date.parse(b.published_at) - Date.parse(a.published_at));
  if (!latest) return undefined;

  const version = latest.tag_name.slice(TAG_PREFIX.length);
  const bundle = assetUrl(latest, assetName(version));
  const png = assetUrl(latest, assetName(version, "png"));
  const gif = assetUrl(latest, assetName(version, "gif"));
  const webp = assetUrl(latest, assetName(version, "webp"));
  const avif = assetUrl(latest, assetName(version, "avif"));
  const jxl = assetUrl(latest, assetName(version, "jxl"));
  if (!bundle || !png || !gif || !webp || !avif || !jxl) return undefined;

  return {
    version,
    date: new Date(latest.published_at).toISOString(),
    bundle,
    assets: { png, gif, webp, avif, jxl },
  };
}

export async function getRelease(config: Config): Promise<Release | undefined> {
  const { data: releases } = await octokit.rest.repos.listReleases({
    owner: config.owner,
    repo: config.repo,
    per_page: 100,
  });

  return selectRelease(releases);
}

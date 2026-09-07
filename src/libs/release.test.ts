import assert from "node:assert/strict";
import { describe, test } from "node:test";
import { type ReleaseData, selectRelease } from "./release.ts";

const BASE_ID = "aviutl2-animated-image-output";

function assets(names: string[]): ReleaseData["assets"] {
  return names.map((name) => ({
    name,
    browser_download_url: `https://example.test/${name}`,
  }));
}

function packageNames(version: string): string[] {
  return [
    `${BASE_ID}-v${version}.au2pkg.zip`,
    `${BASE_ID}-png-v${version}.au2pkg.zip`,
    `${BASE_ID}-gif-v${version}.au2pkg.zip`,
    `${BASE_ID}-webp-v${version}.au2pkg.zip`,
    `${BASE_ID}-avif-v${version}.au2pkg.zip`,
    `${BASE_ID}-jxl-v${version}.au2pkg.zip`,
  ];
}

function release(
  tag: string,
  publishedAt: string | null,
  names: string[],
  flags: { draft?: boolean; prerelease?: boolean } = {},
): ReleaseData {
  return {
    tag_name: tag,
    published_at: publishedAt,
    draft: flags.draft ?? false,
    prerelease: flags.prerelease ?? false,
    assets: assets(names),
  };
}

describe("selectRelease", () => {
  test("6 本揃った版タグから全部入りと形式ごとの URL を引く", () => {
    const selected = selectRelease([
      release("v2.0.0", "2026-09-01T00:00:00Z", packageNames("2.0.0")),
    ]);

    assert.deepEqual(selected, {
      version: "2.0.0",
      date: "2026-09-01T00:00:00.000Z",
      bundle: `https://example.test/${BASE_ID}-v2.0.0.au2pkg.zip`,
      assets: {
        png: `https://example.test/${BASE_ID}-png-v2.0.0.au2pkg.zip`,
        gif: `https://example.test/${BASE_ID}-gif-v2.0.0.au2pkg.zip`,
        webp: `https://example.test/${BASE_ID}-webp-v2.0.0.au2pkg.zip`,
        avif: `https://example.test/${BASE_ID}-avif-v2.0.0.au2pkg.zip`,
        jxl: `https://example.test/${BASE_ID}-jxl-v2.0.0.au2pkg.zip`,
      },
    });
  });

  test("1 本欠けた版タグは不成立", () => {
    const names = packageNames("2.0.0").filter(
      (name) => !name.includes("-jxl-"),
    );

    assert.equal(
      selectRelease([release("v2.0.0", "2026-09-01T00:00:00Z", names)]),
      undefined,
    );
  });

  test("形式ごとのタグの方が新しくても版タグを採る", () => {
    const selected = selectRelease([
      release("v2.0.0", "2026-09-01T00:00:00Z", packageNames("2.0.0")),
      release("gif-v9.9.9", "2026-09-05T00:00:00Z", ["gif_output.auo2"]),
    ]);

    assert.equal(selected?.version, "2.0.0");
  });

  test("draft と prerelease は除く", () => {
    assert.equal(
      selectRelease([
        release("v2.0.0", "2026-09-01T00:00:00Z", packageNames("2.0.0"), {
          draft: true,
        }),
        release("v1.9.0", "2026-08-01T00:00:00Z", packageNames("1.9.0"), {
          prerelease: true,
        }),
      ]),
      undefined,
    );
  });

  test("公開が新しい方を採る", () => {
    const selected = selectRelease([
      release("v2.0.0", "2026-09-01T00:00:00Z", packageNames("2.0.0")),
      release("v2.1.0", "2026-09-05T00:00:00Z", packageNames("2.1.0")),
    ]);

    assert.equal(selected?.version, "2.1.0");
  });

  test("新しい方が 6 本揃わなければ旧い版へ遡らない", () => {
    const selected = selectRelease([
      release("v2.0.0", "2026-09-01T00:00:00Z", packageNames("2.0.0")),
      release("v2.1.0", "2026-09-05T00:00:00Z", [
        `${BASE_ID}-v2.1.0.au2pkg.zip`,
      ]),
    ]);

    assert.equal(selected, undefined);
  });
});

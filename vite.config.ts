import assert from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { Readable } from "node:stream";
import { pathToFileURL } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import rsc from "@vitejs/plugin-rsc";
import { defineConfig, type Plugin, type ResolvedConfig } from "vite";
import inspect from "vite-plugin-inspect";
import tsconfigPaths from "vite-tsconfig-paths";
import { RSC_POSTFIX } from "./src/framework/shared";

export default defineConfig((env) => ({
  plugins: [
    react(),
    rsc({
      entries: {
        client: "./src/framework/entry.browser.tsx",
        rsc: "./src/framework/entry.rsc.tsx",
        ssr: "./src/framework/entry.ssr.tsx",
      },
      serverHandler: env.isPreview ? false : undefined,
    }),
    rscSsgPlugin(),
    inspect(),
    tsconfigPaths(),
    tailwindcss(),
  ],
}));

function rscSsgPlugin(): Plugin[] {
  return [
    {
      name: "rsc-ssg",
      config(_config, env) {
        if (env.isPreview) {
          return {
            appType: "mpa",
          };
        }
      },
      // Use post ssr writeBundle to wait for app is fully built.
      // On Vite 7, you can use `buildApp` hook instead.
      writeBundle: {
        order: "post",
        async handler() {
          if (this.environment.name === "ssr") {
            const config = this.environment.getTopLevelConfig();
            await renderStatic(config);
          }
        },
      },
    },
  ];
}

async function renderStatic(config: ResolvedConfig) {
  // import server entry
  const entryPath = path.join(config.environments.rsc.build.outDir, "index.js");
  const entry: typeof import("./src/framework/entry.rsc") = await import(
    pathToFileURL(entryPath).href
  );

  // entry provides a list of static paths
  const staticPaths = await entry.getStaticPaths();

  // render rsc and html
  const baseDir = config.environments.client.build.outDir;
  for (const htmlPath of staticPaths) {
    config.logger.info(`[vite-rsc:ssg] -> ${htmlPath}`);
    const rscPath = htmlPath + RSC_POSTFIX;
    const htmlResponse = await entry.default(
      new Request(new URL(htmlPath, "http://ssg.local")),
    );
    assert.equal(htmlResponse.status, 200);
    await fs.promises.writeFile(
      path.join(baseDir, normalizeHtmlFilePath(htmlPath)),
      // biome-ignore lint/suspicious/noExplicitAny: ignore
      Readable.fromWeb(htmlResponse.body as any),
    );

    const rscResponse = await entry.default(
      new Request(new URL(rscPath, "http://ssg.local")),
    );
    assert.equal(rscResponse.status, 200);
    await fs.promises.writeFile(
      path.join(baseDir, rscPath),
      // biome-ignore lint/suspicious/noExplicitAny: ignore
      Readable.fromWeb(rscResponse.body as any),
    );
  }
}

function normalizeHtmlFilePath(p: string) {
  if (p.endsWith("/")) {
    return `${p}index.html`;
  }
  return `${p}.html`;
}

export const PLUGINS = ["png", "gif", "webp", "avif", "jxl"] as const;

export type Plugin = (typeof PLUGINS)[number];

export type Release = {
  version: string;
  date: string;
  bundle: string;
  assets: Record<Plugin, string>;
};

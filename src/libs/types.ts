export type Release = {
  version: string;
  date: string;
  url: string;
};

export type Plugin = "png" | "gif" | "webp" | "avif";

export type PluginRelease = {
  [key in Plugin]?: Release;
};

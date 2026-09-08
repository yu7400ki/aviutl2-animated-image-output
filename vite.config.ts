import path from "node:path";

import funstackStatic from "@funstack/static";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  resolve: {
    tsconfigPaths: true,
  },
  base: process.env.BASE_URL ?? "/",
  plugins: [
    funstackStatic({
      root: "./src/root.tsx",
      app: "./src/app.tsx",
      ssr: true,
      publicOutDir: path.join("dist/client", process.env.BASE_URL ?? "/"),
    }),
    react(),
    tailwindcss(),
  ],
});

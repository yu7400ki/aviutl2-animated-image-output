import { defineConfig } from "oxlint";

export default defineConfig({
  plugins: ["typescript", "unicorn", "oxc", "react", "import"],
  categories: { correctness: "error", perf: "warn", suspicious: "error" },
  rules: {
    "no-console": "off",
    "unicorn/filename-case": ["error", { case: "kebabCase" }],
    "react/react-in-jsx-scope": "off",
    "import/no-unassigned-import": ["error", { allow: ["**/*.css"] }],
  },
  overrides: [
    {
      files: ["src/**"],
      excludeFiles: ["src/**/*.test.ts"],
      rules: { "import/no-nodejs-modules": "error" },
    },
  ],
  ignorePatterns: ["**/vendor/**"],
  env: { builtin: true },
});

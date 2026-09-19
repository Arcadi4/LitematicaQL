import { defineConfig } from "vite-plus";
import { viteSingleFile } from "vite-plugin-singlefile";

export default defineConfig({
  plugins: [viteSingleFile()],
  // `vendor/pack.zip` is the tracked Minecraft resource pack. Serving it as the
  // public directory copies it beside the bundle without a staging step.
  publicDir: "vendor",
  build: {
    emptyOutDir: true,
    modulePreload: false,
    outDir: "../Resources/Renderer",
    sourcemap: false,
    target: "safari16",
  },
  fmt: {},
  lint: {
    jsPlugins: [
      {
        name: "vite-plus",
        specifier: "vite-plus/oxlint-plugin",
      },
    ],
    options: {
      typeAware: true,
      typeCheck: true,
    },
    rules: {
      "vite-plus/prefer-vite-plus-imports": "error",
    },
  },
});

import { cloudflare } from "@cloudflare/vite-plugin";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [cloudflare()],
  publicDir: "../build/update-assets",
  build: { outDir: "../build/update-vite" },
});

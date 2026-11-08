import { defineConfig } from "cf/config";

export default defineConfig({
  accountId: "f4e323f273bab9cb104cbe7b945c58d3",
  worker: {
    name: "apps-update",
    compatibilityDate: "2026-11-07",
    workersDev: false,
    domains: ["update.arcadia.moe"],
  },
});

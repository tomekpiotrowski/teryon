import { defineConfig } from "@playwright/test";
import { join } from "node:path";

// The app under test runs on a port of its own, so a development server on
// the usual port, perhaps serving another branch, is never tested by mistake.
const appUrl = process.env.APP_URL ?? "http://127.0.0.1:4310";
const appPort = new URL(appUrl).port || "4310";

// Every test starts `teryon service` itself, see `support.ts`. Build it first:
// `cargo build -p teryon` at the top of the repository.
export default defineConfig({
  testDir: ".",
  timeout: 30_000,
  use: {
    baseURL: appUrl,
    ...(process.env.PLAYWRIGHT_CHROMIUM_PATH
      ? { launchOptions: { executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH } }
      : {}),
  },
  // The app is started through bun, which is what the project installs with.
  webServer: process.env.SKIP_APP_SERVER
    ? undefined
    : {
        command: `bun run start -- --port ${appPort} --host 127.0.0.1`,
        cwd: join(__dirname, "..", "app"),
        url: appUrl,
        reuseExistingServer: !process.env.CI,
        timeout: 180_000,
      },
});

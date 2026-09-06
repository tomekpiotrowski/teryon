import { defineConfig } from "@playwright/test";
import { join } from "node:path";

const api = process.env.API_URL ?? "http://127.0.0.1:3001";
const appUrl = process.env.APP_URL ?? "http://127.0.0.1:4200";
const appPort = new URL(appUrl).port || "4200";

export default defineConfig({
  testDir: ".",
  timeout: 30_000,
  use: {
    baseURL: appUrl,
    ...(process.env.PLAYWRIGHT_CHROMIUM_PATH
      ? { launchOptions: { executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH } }
      : {}),
  },
  // `erno test --e2e` starts the API on a free port and sets API_URL / APP_URL.
  //
  // The app is started through bun, which is what the project installs with —
  // `npm` on a bun-managed machine is bun's shim, and it ignores `--prefix`,
  // so the old command looked for a `start` script in e2e/ and found none.
  // Point the spawned process at app/ instead of asking the package manager to.
  webServer: process.env.SKIP_APP_SERVER
    ? undefined
    : {
        command: `bun run start -- --port ${appPort} --host 127.0.0.1`,
        cwd: join(__dirname, "..", "app"),
        url: appUrl,
        reuseExistingServer: !process.env.ERNO_E2E && !process.env.CI,
        timeout: 180_000,
      },
  metadata: { api },
});

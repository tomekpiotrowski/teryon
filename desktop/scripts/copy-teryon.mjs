// Puts the freshly built `teryon` program where Tauri's bundler looks for it.
//
// Tauri ships extra programs named after the platform they were built for,
// as `binaries/teryon-<target>`, and strips that name again in the bundle.
// Run from `app/` by the build command in `tauri.conf.json`, with the cargo
// profile to copy from as its argument: `release` (the default) or `debug`.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const root = join(import.meta.dirname, "..", "..");
const target = execFileSync("rustc", ["--print", "host-tuple"], { encoding: "utf8" }).trim();
const extension = process.platform === "win32" ? ".exe" : "";
const folder = join(root, "desktop", "src-tauri", "binaries");
mkdirSync(folder, { recursive: true });
const profile = process.argv[2] ?? "release";
copyFileSync(join(root, "target", profile, `teryon${extension}`), join(folder, `teryon-${target}${extension}`));

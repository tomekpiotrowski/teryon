// What the end-to-end specs share: a Teryon service of their own, a way to
// use the menu, a way to add a project, and a way to make its directory a git
// repository.
//
// Every test starts `teryon service` in a temporary home folder, configured
// to let the app under test connect and to start a script that replays
// recordings in place of the agent programs. Nothing here reaches a model
// provider, and nothing touches the projects of whoever runs the tests.

import { expect, test as base, type Locator, type Page } from "@playwright/test";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { createInterface } from "node:readline";

const root = join(__dirname, "..");

/** The `teryon` program, built before the tests run. */
const program = process.env.TERYON_PROGRAM ?? join(root, "target", "debug", "teryon");

/** The recordings the fake agents replay. */
const samples = join(root, "core", "tests", "samples");

/** The app under test, as the service sees it. */
function appOrigin(baseURL: string | undefined): string {
  return new URL(baseURL ?? "http://127.0.0.1:4310").origin;
}

/** A running service, and the address that opens the app connected to it. */
interface Service {
  readonly address: string;
  readonly home: string;
  readonly process: ChildProcess;
}

/** Starts a service in a fresh home folder, with `more` added to its settings, and waits for the address it prints. */
async function startService(origin: string, more: string): Promise<Service> {
  const home = mkdtempSync(join(tmpdir(), "teryon-e2e-"));
  mkdirSync(join(home, "config"), { recursive: true });
  writeFileSync(
    join(home, "config", "config.toml"),
    [
      "[service]",
      "port = 7400",
      `allowed_origins = ["${origin}"]`,
      "",
      "[agents]",
      "stop_grace_seconds = 1",
      "",
      "[agents.claude]",
      `program = "${join(samples, "fake-claude.sh")}"`,
      "",
      "[agents.codex]",
      `program = "${join(samples, "fake-codex.sh")}"`,
      // Never the real `~/.codex`: tests must not read a person's Codex sessions.
      `home = "${join(home, "codex")}"`,
      "",
      "[agents.grok]",
      `program = "${join(samples, "fake-grok.sh")}"`,
      "",
      "[agents.gemini]",
      `program = "${join(samples, "fake-gemini.sh")}"`,
      "",
      more,
    ].join("\n"),
  );
  const child = spawn(program, ["--home", home, "service", "--print-address", `${origin}/`], {
    stdio: ["ignore", "pipe", "inherit"],
  });
  const lines = createInterface({ input: child.stdout! });
  const address = await new Promise<string>((resolve, reject) => {
    child.once("exit", (code) => reject(new Error(`teryon service ended early (${code})`)));
    lines.once("line", resolve);
  });
  return { address, home, process: child };
}

/** Stops a service and waits for it to end. */
async function stopService(service: Service): Promise<void> {
  if (service.process.exitCode !== null) return;
  const ended = new Promise((resolve) => service.process.once("exit", resolve));
  service.process.kill("SIGTERM");
  await ended;
}

/**
 * Playwright's `test`, with a service of its own for every test.
 *
 * `settings` is added to the service's `config.toml`, for a test that needs
 * more than the fake agents: `test.use({ settings: "[stages.review]\n…" })`.
 */
export const test = base.extend<{ service: Service; settings: string }>({
  settings: ["", { option: true }],
  service: async ({ baseURL, settings }, use) => {
    const service = await startService(appOrigin(baseURL), settings);
    await use(service);
    await stopService(service);
  },
});

export { expect };

/** Where a test's project directories go. */
const projectsFolder = join(tmpdir(), "teryon-e2e-projects");

/**
 * Adds a project in a fresh directory and returns the directory once the
 * project is selected. `prepare` fills the directory before Teryon sees it.
 */
export async function addProject(page: Page, name: string, prepare?: (directory: string) => void): Promise<string> {
  mkdirSync(projectsFolder, { recursive: true });
  const directory = mkdtempSync(join(projectsFolder, "e2e-"));
  prepare?.(directory);

  await choose(page, "New project");
  await page.getByLabel("Project name").fill(name);
  await page.getByLabel("Project directory").fill(directory);
  await page.getByRole("button", { name: "Add project" }).click();
  await expect(page.getByPlaceholder(`What should Claude do in ${name}?`)).toBeVisible();
  return directory;
}

/** The menu, which is how a person gets around the workspace. */
export function menu(page: Page): Locator {
  return page.getByRole("dialog", { name: "Menu" });
}

/** Opens the menu with its key and picks the command whose name starts with the given text. */
export async function choose(page: Page, command: string): Promise<void> {
  await page.keyboard.press("Control+k");
  await menu(page).getByRole("option", { name: new RegExp(`^${command}`) }).click();
}

/** Opens the app connected to the test's service, on the empty workspace. For `test.beforeEach`. */
export async function openWorkspace({ page, service }: { page: Page; service: Service }): Promise<void> {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto(service.address);
  const minimize = page.getByRole("dialog", { name: "Erno Devtools" }).getByRole("button", { name: "—" });
  if (await minimize.isVisible()) await minimize.click();
  await expect(page.getByText("Add a project to get started.")).toBeVisible();
}

/**
 * Runs git in a directory and returns what it printed, without the line ending.
 *
 * With `time`, in seconds since 1970, git runs as if it were that moment, so
 * a commit made this way carries it as the time it was made.
 */
export function git(directory: string, ...parameters: string[]): string {
  return runGit(directory, undefined, parameters);
}

export function gitAt(directory: string, time: number, ...parameters: string[]): string {
  return runGit(directory, time, parameters);
}

function runGit(directory: string, time: number | undefined, parameters: string[]): string {
  const moment = time === undefined ? {} : { GIT_AUTHOR_DATE: `${time} +0000`, GIT_COMMITTER_DATE: `${time} +0000` };
  return execFileSync("git", parameters, {
    cwd: directory,
    stdio: "pipe",
    encoding: "utf8",
    env: {
      ...process.env,
      // The repositories ignore the settings of whoever runs the tests.
      GIT_CONFIG_GLOBAL: "/dev/null",
      GIT_CONFIG_SYSTEM: "/dev/null",
      GIT_AUTHOR_NAME: "Tester",
      GIT_AUTHOR_EMAIL: "tester@example.com",
      GIT_COMMITTER_NAME: "Tester",
      GIT_COMMITTER_EMAIL: "tester@example.com",
      ...moment,
    },
  }).trim();
}

/** Writes a file in a directory, making the folders on the way. */
export function put(directory: string, path: string, content: string): void {
  mkdirSync(dirname(join(directory, path)), { recursive: true });
  writeFileSync(join(directory, path), content);
}

/** Commits everything in a directory as it is. */
export function commit(directory: string): void {
  git(directory, "add", "-A");
  git(directory, "commit", "-q", "-m", "change");
}

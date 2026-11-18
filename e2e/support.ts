// What the end-to-end specs share: a signed-in person, a way to use the menu,
// a way to add a project, and a way to make its directory a git repository.
//
// The API runs with the test configuration, which replaces the agent programs
// with a script that replays recordings. Nothing here reaches a model provider.

import { expect, type APIRequestContext, type Locator, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const api = process.env.API_URL ?? "http://127.0.0.1:3001";

/** The one folder the test configuration allows projects in. */
const projectsFolder = join(__dirname, "..", "api", "target", "test-projects");

const password = "a long password";

interface MockEmail {
  to: string;
  body_html: string | null;
  body_text: string | null;
}

/**
 * Registers a person and confirms their address the way they would: with the
 * link from the verification email, read from the API's development inbox.
 */
async function registeredPerson(request: APIRequestContext): Promise<string> {
  const email = `person-${Date.now()}-${Math.floor(Math.random() * 1e6)}@example.com`;
  const registration = await request.post(`${api}/api/auth/register`, { data: { email, password } });
  expect(registration.ok()).toBeTruthy();

  let token: string | undefined;
  await expect
    .poll(
      async () => {
        const inbox: MockEmail[] = await (await request.get(`${api}/dev/emails`)).json();
        const message = inbox.find((candidate) => candidate.to === email);
        token = `${message?.body_html ?? ""} ${message?.body_text ?? ""}`.match(/verify-email\?token=([\w.-]+)/)?.[1];
        return token;
      },
      { timeout: 20_000 },
    )
    .toBeTruthy();

  const verification = await request.post(`${api}/api/auth/email/verify`, { data: { token } });
  expect(verification.ok()).toBeTruthy();
  return email;
}

async function collapseDevtools(page: Page): Promise<void> {
  const minimize = page.getByRole("dialog", { name: "Erno Devtools" }).getByRole("button", { name: "—" });
  if (await minimize.isVisible()) await minimize.click();
}

async function signIn(page: Page, email: string): Promise<void> {
  await page.goto("/login");
  await collapseDevtools(page);
  await page.getByLabel("Work email").fill(email);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: /Sign in/ }).click();
  await expect(page.getByText("Add a project to get started.")).toBeVisible();
}

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

/** Signs a new person in and leaves them on the empty workspace. For `test.beforeEach`. */
export async function openWorkspace({ page, request }: { page: Page; request: APIRequestContext }): Promise<void> {
  // The API under test listens on a port chosen for this run; the application
  // would otherwise look for it where the development server usually is.
  await page.addInitScript((url) => {
    (window as { __ERNO_API_URL__?: string }).__ERNO_API_URL__ = url;
  }, api);
  await page.setViewportSize({ width: 1280, height: 800 });
  await signIn(page, await registeredPerson(request));
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

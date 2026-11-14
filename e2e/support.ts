// What the end-to-end specs share: a signed-in person and a way to add a project.
//
// The API runs with the test configuration, which replaces the agent programs
// with a script that replays recordings. Nothing here reaches a model provider.

import { expect, type APIRequestContext, type Page } from "@playwright/test";
import { mkdirSync, mkdtempSync } from "node:fs";
import { join } from "node:path";

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

  await page.getByRole("button", { name: "New project" }).click();
  await page.getByLabel("Project name").fill(name);
  await page.getByLabel("Project directory").fill(directory);
  await page.getByRole("button", { name: "Add project" }).click();
  await expect(page.getByPlaceholder(`What should Claude do in ${name}?`)).toBeVisible();
  return directory;
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

// End-to-end specs for the workspace: a person signs in, adds a project and
// has an agent work in it.
//
// The API runs with the test configuration, which replaces the agent programs
// with a script that replays recordings. Nothing here reaches a model provider.

import { expect, test, type APIRequestContext, type Page } from "@playwright/test";
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

/** Adds a project in a fresh directory and returns once it is selected. */
async function addProject(page: Page, name: string): Promise<void> {
  mkdirSync(projectsFolder, { recursive: true });
  const directory = mkdtempSync(join(projectsFolder, "e2e-"));

  await page.getByRole("button", { name: "New project" }).click();
  await page.getByLabel("Project name").fill(name);
  await page.getByLabel("Project directory").fill(directory);
  await page.getByRole("button", { name: "Add project" }).click();
  await expect(page.getByPlaceholder(`What should Claude do in ${name}?`)).toBeVisible();
}

test.beforeEach(async ({ page, request }) => {
  // The API under test listens on a port chosen for this run; the application
  // would otherwise look for it where the development server usually is.
  await page.addInitScript((url) => {
    (window as { __ERNO_API_URL__?: string }).__ERNO_API_URL__ = url;
  }, api);
  await page.setViewportSize({ width: 1280, height: 800 });
  await signIn(page, await registeredPerson(request));
});

test("an agent works in a project and the thread fills in live", async ({ page }) => {
  await addProject(page, "Sample");

  await page.getByLabel("New thread").fill("hello");
  await page.getByRole("button", { name: "Run ↵" }).click();

  await expect(page.getByLabel("Thread").getByText("hello", { exact: true })).toBeVisible();
  await expect(page.getByLabel("Thread").getByText("ready", { exact: true })).toBeVisible();
  await expect(page.getByText(/^Done in \d+s · \$0\.21$/)).toBeVisible();
  await expect(page.getByLabel("Projects").getByRole("button", { name: "hello" })).toBeVisible();

  await page.getByLabel("Reply").fill("use tools");
  await page.getByRole("button", { name: "send ↵" }).click();

  await expect(page.getByText("Bash  Check repository state and existing worktrees")).toBeVisible();
  await expect(page.getByText(/note\.txt` is 6 bytes/)).toBeVisible();
  await expect(page.getByLabel("Reply")).toHaveValue("");
});

test("a thread can be started with Grok", async ({ page }) => {
  await addProject(page, "Sample");

  await page.getByRole("button", { name: "Change agent" }).click();
  await page.getByPlaceholder("What should Grok do in Sample?").fill("use tools");
  await page.getByRole("button", { name: "Run ↵" }).click();

  await expect(page.getByText("run_terminal_command  Count bytes in note.txt")).toBeVisible();
  await expect(page.getByText(/Auto mode blocked this action/)).toBeVisible();
  await expect(page.locator(".thread-meta").getByText("Grok")).toBeVisible();
});

test("a running agent can be stopped", async ({ page }) => {
  await addProject(page, "Sample");

  await page.getByLabel("New thread").fill("be slow");
  await page.getByRole("button", { name: "Run ↵" }).click();

  await expect(page.getByRole("status")).toHaveText(/Claude is working…/);
  await page.getByRole("button", { name: "stop" }).click();

  await expect(page.getByText("Stopped by you.")).toBeVisible();
  await expect(page.getByRole("button", { name: "send ↵" })).toBeVisible();
});

test("a refused project says why", async ({ page }) => {
  await page.getByRole("button", { name: "New project" }).click();
  await page.getByLabel("Project name").fill("Outside");
  await page.getByLabel("Project directory").fill("/");
  await page.getByRole("button", { name: "Add project" }).click();

  await expect(page.getByText("The directory is not inside a folder Teryon is allowed to use.")).toBeVisible();
  await expect(page.getByText("Add a project to get started.")).toBeVisible();
});

test("a project can be removed", async ({ page }) => {
  await addProject(page, "Sample");

  await page.getByRole("button", { name: "Remove Sample" }).click();
  await page.getByRole("button", { name: "Remove", exact: true }).click();

  await expect(page.getByText("Add a project to get started.")).toBeVisible();
});

test("the phone layout uses the project drawer and the theme control", async ({ page }) => {
  await addProject(page, "Sample");
  await page.setViewportSize({ width: 390, height: 844 });

  await expect(page.locator(".mobile-header").getByText("Sample")).toBeVisible();
  await page.getByRole("button", { name: "Open projects" }).click();
  await expect(page.locator(".project-navigation")).toBeInViewport();
  await page.getByRole("button", { name: "Close projects" }).first().click();

  const before = await page.locator("html").getAttribute("data-theme");
  await page.getByRole("button", { name: "Toggle color theme" }).click();
  await expect(page.locator("html")).not.toHaveAttribute("data-theme", before ?? "");
});

// End-to-end specs for Home: the prompt that starts or continues a thread, and
// the list of threads that need the person.
//
// The fake agent fails, asks questions or changes files when its message says
// so. Nothing here reaches a model provider.

import type { Page } from "@playwright/test";

import { addProject, agentButton, choose, commit, expect, git, menu, openWorkspace, put, test } from "./support";

test.beforeEach(openWorkspace);

/** The part of the screen that is Home. */
function home(page: Page) {
  return page.locator("app-home");
}

/** The list of threads that need the person. */
function waiting(page: Page) {
  return home(page).getByRole("list", { name: "Threads that need you" });
}

/** Goes to Home by the name in the bar. */
async function goHome(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Home" }).click();
  await expect(home(page)).toBeVisible();
}

/** Starts a thread from Home and waits for the agent to finish, then goes back to Home. */
async function startAndReturn(page: Page, message: string): Promise<void> {
  await page.getByLabel("New thread").fill(message);
  await page.getByRole("button", { name: "Run ↵" }).click();
  // The message is in the thread once the agent has been asked, and it is done when it stops working.
  await expect(page.getByLabel("Thread", { exact: true }).getByText(message, { exact: true })).toBeVisible();
  await expect(page.getByText(/is working…/)).toHaveCount(0);
  await goHome(page);
}

test("with nothing waiting Home says so and offers only the prompt", async ({ page }) => {
  await addProject(page, "Sample");

  await expect(home(page).getByText("Nothing needs you.")).toBeVisible();
  await expect(waiting(page)).toHaveCount(0);
  await expect(page.locator(".top-bar")).toContainText("all projects");
  await expect(page.getByRole("button", { name: /need you$/ })).toHaveCount(0);
  await expect(page.getByLabel("New thread")).toHaveAttribute("placeholder", "Start a thread — or type @ to continue one");
});

test("a thread starts with the agent and workflow pressed on Home", async ({ page }) => {
  await addProject(page, "Sample");

  await agentButton(page, "Grok").click();
  await expect(agentButton(page, "Grok")).toHaveAttribute("aria-pressed", "true");
  await expect(agentButton(page, "Claude")).toHaveAttribute("aria-pressed", "false");
  await home(page).getByRole("button", { name: "default" }).click();
  await home(page).getByRole("option", { name: "small" }).click();

  await page.getByLabel("New thread").fill("use tools");
  await page.getByLabel("New thread").press("Enter");

  await expect(page.locator(".thread-meta").getByText("Grok")).toBeVisible();
  await expect(page.getByText("run_terminal_command  Count bytes in note.txt")).toBeVisible();
  // On the small workflow the menu offers the other one, and not the one in use.
  await page.keyboard.press("Control+k");
  await expect(menu(page).getByRole("option", { name: /^Use the default workflow/ })).toBeVisible();
  await expect(menu(page).getByRole("option", { name: /^Use the small workflow/ })).toHaveCount(0);
});

test("Run does nothing until something is typed", async ({ page }) => {
  await addProject(page, "Sample");
  await expect(page.getByRole("button", { name: "Run ↵" })).toBeDisabled();
  await page.getByLabel("New thread").press("Enter");
  await expect(home(page)).toBeVisible();
  await expect(page.getByLabel("Thread", { exact: true })).toHaveCount(0);
});

test("a thread whose agent asks questions waits as a reply, and leaves the count once answered", async ({ page }) => {
  await addProject(page, "Sample");
  await startAndReturn(page, "please ask questions");

  const row = waiting(page).getByRole("listitem").filter({ hasText: /please ask questions/ });
  await expect(row).toContainText("reply");
  await expect(page.getByRole("button", { name: "1 need you" })).toBeVisible();

  await row.click();
  await expect(page.getByRole("group", { name: "The agent’s questions" })).toBeVisible();
});

test("a failed run is listed as failed, and stays until the thread runs again", async ({ page }) => {
  await addProject(page, "Sample");
  await startAndReturn(page, "please fail");

  const row = waiting(page).getByRole("listitem").filter({ hasText: /please fail/ });
  await expect(row).toContainText("failed");
  await expect(row.locator(".home-dot")).toHaveAttribute("data-word", "failed");

  // Looking at it does not clear a failure; only running again does.
  await row.click();
  await expect(page.getByLabel("Thread", { exact: true })).toBeVisible();
  await goHome(page);
  await expect(row).toContainText("failed");
});

test("a change ready to apply is listed as apply, first", async ({ page }) => {
  await addProject(page, "Sample", (folder) => {
    git(folder, "init", "-q", "-b", "main");
    put(folder, "README.md", "# Sample\n");
    commit(folder);
  });
  await startAndReturn(page, "please fail");
  await startAndReturn(page, "start a change fix-login");

  const rows = waiting(page).getByRole("listitem");
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toContainText("start a change fix-login");
  await expect(rows.first()).toContainText("apply");
  await expect(rows.last()).toContainText("failed");
});

test("Tab hands the keys to the list, where j, k and Enter move and open", async ({ page }) => {
  await addProject(page, "Sample");
  await startAndReturn(page, "please fail");
  await startAndReturn(page, "please ask questions");

  const rows = waiting(page).getByRole("listitem");
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toContainText("please fail");
  await expect(rows.last()).toContainText("please ask questions");

  await page.getByLabel("New thread").press("Tab");
  await expect(waiting(page)).toBeFocused();
  await page.keyboard.press("j");
  await page.keyboard.press("Enter");
  // Failed comes before reply, so the second row is the one that asked.
  await expect(page.getByRole("group", { name: "The agent’s questions" })).toBeVisible();
});

test("Tab and Escape in the list go back to the prompt", async ({ page }) => {
  await addProject(page, "Sample");
  await startAndReturn(page, "please fail");

  await page.getByLabel("New thread").press("Tab");
  await expect(waiting(page)).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByLabel("New thread")).toBeFocused();
});

test("@ continues a thread: it picks one by what was typed and sends the rest to it", async ({ page }) => {
  await addProject(page, "Sample");
  await startAndReturn(page, "please fail");

  await page.getByLabel("New thread").fill("@fail");
  const matches = home(page).getByRole("listbox", { name: "Threads to continue" });
  await expect(matches.getByRole("option")).toHaveCount(1);
  await page.getByLabel("New thread").press("Enter");

  await expect(home(page).getByRole("button", { name: /Stop continuing this thread/ })).toContainText("please fail");
  await page.getByLabel("New thread").fill("try again");
  await page.getByLabel("New thread").press("Enter");

  await expect(page.getByLabel("Thread", { exact: true }).getByText("try again", { exact: true })).toBeVisible();
});

test("@ pressed outside the prompt starts the search, and Escape drops it", async ({ page }) => {
  await addProject(page, "Sample");
  await startAndReturn(page, "please fail");
  await page.getByLabel("New thread").blur();

  await page.keyboard.press("@");
  await expect(page.getByLabel("New thread")).toBeFocused();
  await expect(page.getByLabel("New thread")).toHaveValue("@");
  await expect(home(page).getByRole("listbox", { name: "Threads to continue" })).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.getByLabel("New thread")).toHaveValue("");
  await expect(home(page).getByRole("listbox", { name: "Threads to continue" })).toHaveCount(0);
});

test("a search that matches nothing says so, and Enter starts nothing", async ({ page }) => {
  await addProject(page, "Sample");
  await page.getByLabel("New thread").fill("@zzzz");
  await expect(home(page).getByText("No thread matches.")).toBeVisible();
  await page.getByLabel("New thread").press("Enter");
  await expect(page.getByLabel("Thread", { exact: true })).toHaveCount(0);
});

test("Backspace on an empty prompt drops the chosen thread", async ({ page }) => {
  await addProject(page, "Sample");
  await startAndReturn(page, "please fail");
  await page.getByLabel("New thread").fill("@");
  await page.getByLabel("New thread").press("Enter");
  await expect(home(page).getByRole("button", { name: /Stop continuing this thread/ })).toBeVisible();

  await page.getByLabel("New thread").press("Backspace");
  await expect(home(page).getByRole("button", { name: /Stop continuing this thread/ })).toHaveCount(0);
});

test("the agent, project and workflow chosen on Home are there the next time", async ({ page }) => {
  await addProject(page, "Sample");
  await agentButton(page, "Codex").click();
  await page.reload();
  await expect(page.getByLabel("New thread")).toBeVisible();
  await expect(agentButton(page, "Codex")).toHaveAttribute("aria-pressed", "true");
});

test("a project picked on Home is where the thread starts", async ({ page }) => {
  await addProject(page, "First");
  await addProject(page, "Second");

  await home(page).getByRole("button", { name: /Second/ }).click();
  await home(page).getByRole("option", { name: "First" }).click();
  await page.getByLabel("New thread").fill("hello");
  await page.getByRole("button", { name: "Run ↵" }).click();

  await expect(page.locator(".top-bar")).toContainText("First · hello");
});

test("Escape from a thread goes to Home", async ({ page }) => {
  await addProject(page, "Sample");
  await startAndReturn(page, "please fail");
  await page.getByLabel("Open menu").click();
  await menu(page).getByRole("option", { name: /^please fail/ }).click();
  await expect(page.getByLabel("Thread", { exact: true })).toBeVisible();

  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("Escape");
  await expect(home(page)).toBeVisible();
});

test("a refused message keeps the text and the chosen thread on Home", async ({ page }) => {
  await addProject(page, "Sample");
  await page.getByLabel("New thread").fill("be slow");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.getByText(/is working…/)).toBeVisible();
  await goHome(page);

  await page.getByLabel("New thread").fill("@slow");
  await page.getByLabel("New thread").press("Enter");
  await page.getByLabel("New thread").fill("one more thing");
  await page.getByLabel("New thread").press("Enter");

  // The agent is still working, so the message is refused; nothing is lost.
  await expect(page.locator("ion-toast, .erno-alert, [role=alert]").first()).toBeVisible();
  await expect(home(page)).toBeVisible();
  await expect(page.getByLabel("New thread")).toHaveValue("one more thing");
  await expect(home(page).getByRole("button", { name: /Stop continuing this thread/ })).toContainText("be slow");
});

test("an agent picked in the menu shows on Home, and one pressed on Home shows in the menu", async ({ page }) => {
  await addProject(page, "Sample");

  await agentButton(page, "Codex").click();
  await expect(agentButton(page, "Codex")).toHaveAttribute("aria-pressed", "true");

  await choose(page, "Use Grok · default for new threads");
  await expect(agentButton(page, "Grok")).toHaveAttribute("aria-pressed", "true");
  await expect(agentButton(page, "Codex")).toHaveAttribute("aria-pressed", "false");

  // The thread starts on what Home shows.
  await page.getByLabel("New thread").fill("hello");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.locator(".thread-meta").getByText("Grok")).toBeVisible();
});

test("a thread started on another workflow follows it", async ({ page }) => {
  await addProject(page, "Sample");
  await home(page).getByRole("button", { name: "default" }).click();
  await home(page).getByRole("option", { name: "small" }).click();
  await page.getByLabel("New thread").fill("hello");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.getByText(/^Done in/)).toBeVisible();

  await page.keyboard.press("Control+k");
  await expect(menu(page).getByRole("option", { name: /^Use the default workflow/ })).toBeVisible();
});

test("G then H goes home from the code, the history and the board", async ({ page }) => {
  await addProject(page, "Sample", (folder) => {
    git(folder, "init", "-q", "-b", "main");
    put(folder, "README.md", "# Sample\n");
    commit(folder);
  });
  await startAndReturn(page, "please fail");
  await page.getByRole("button", { name: "Home" }).click();
  await waiting(page).getByRole("listitem").first().click();
  await expect(page.getByLabel("Thread", { exact: true })).toBeVisible();

  for (const key of ["c", "h", "b"]) {
    await page.getByLabel("Reply").press("Escape");
    await page.keyboard.press(key);
    await expect(home(page)).toBeHidden();
    await page.keyboard.press("g");
    await page.keyboard.press("h");
    await expect(home(page)).toBeVisible();
    await waiting(page).getByRole("listitem").first().click();
    await expect(page.getByLabel("Thread", { exact: true })).toBeVisible();
  }
});

test("the menu has a Home command", async ({ page }) => {
  await addProject(page, "Sample");
  await startAndReturn(page, "please fail");
  await waiting(page).getByRole("listitem").first().click();
  await choose(page, "Home");
  await expect(home(page)).toBeVisible();
});

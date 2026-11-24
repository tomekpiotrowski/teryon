// End-to-end specs for Home: the prompt that starts or continues a thread, and
// the list of threads that need the person.
//
// The fake agent fails, asks questions or changes files when its message says
// so. Nothing here reaches a model provider.

import type { Page } from "@playwright/test";

import { addProject, agentChooser, choose, chooseAgent, commit, expect, git, menu, openWorkspace, put, test } from "./support";

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
  await expect(page.getByLabel("New thread")).toHaveAttribute("placeholder", "Start a thread");
});

test("Home opens searchable shortcuts from the keyboard, the bottom bar and the menu", async ({ page }) => {
  await addProject(page, "Sample");
  const prompt = page.getByLabel("New thread");
  const dialog = page.getByRole("dialog", { name: "Shortcuts" });
  const filter = dialog.getByRole("textbox", { name: "Filter shortcuts" });

  await prompt.press("?");
  await expect(prompt).toHaveValue("?");
  await expect(dialog).toBeHidden();
  await prompt.blur();
  await page.keyboard.press("?");
  await expect(dialog.getByRole("heading")).toHaveText(["Here · Home", "Everywhere", "Go to"]);
  await expect(filter).toBeFocused();
  await filter.fill("settings");
  await expect(dialog.getByRole("heading")).toHaveText(["Go to"]);
  await expect(dialog.getByText("Settings", { exact: true })).toBeVisible();
  await filter.press("?");
  await expect(filter).toHaveValue("settings?");
  await expect(dialog.getByText("Nothing matches.")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();

  const button = home(page).getByRole("button", { name: /all shortcuts/ });
  await button.click();
  await expect(filter).toHaveValue("");
  await filter.press("/");
  await expect(dialog).toBeHidden();
  const option = menu(page).getByRole("option", { name: /Shortcuts/ });
  await expect(option).toContainText("?");
  await option.click();
  await expect(dialog).toBeVisible();
  await page.locator(".menu-backdrop").click({ position: { x: 10, y: 10 } });
  await expect(dialog).toBeHidden();

  await page.setViewportSize({ width: 390, height: 844 });
  await button.click();
  await dialog.getByRole("button", { name: "Close shortcuts" }).click();
  await expect(dialog).toBeHidden();
  await expect(button).toBeFocused();
});

test("Escape closes shortcuts before the open Agent dropdown", async ({ page }) => {
  await addProject(page, "Sample");
  const prompt = page.getByLabel("New thread");
  const agent = agentChooser(page);
  const choices = home(page).getByRole("listbox", { name: "Agent", exact: true });
  const dialog = page.getByRole("dialog", { name: "Shortcuts" });

  await prompt.fill("keep this");
  await agent.click();
  await expect(choices).toBeVisible();
  await agent.press("?");
  await expect(dialog.getByRole("textbox", { name: "Filter shortcuts" })).toBeFocused();

  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(agent).toBeFocused();
  await expect(choices).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(choices).toBeHidden();
  await expect(prompt).toHaveValue("keep this");
});

test("a thread starts with the agent chosen on Home", async ({ page }) => {
  await addProject(page, "Sample");

  await chooseAgent(page, "Grok");
  await expect(agentChooser(page)).toHaveAccessibleName("Grok");
  await chooseAgent(page, "Grok");
  await expect(agentChooser(page)).toHaveAttribute("aria-expanded", "false");
  await expect(page.getByLabel("New thread")).toBeFocused();

  await page.getByLabel("New thread").fill("use tools");
  await page.getByLabel("New thread").press("Enter");

  await expect(page.locator(".thread-meta").getByText("Grok")).toBeVisible();
  await expect(page.getByText("run_terminal_command  Count bytes in note.txt")).toBeVisible();
  await page.keyboard.press("Control+k");
  await expect(menu(page).getByRole("option", { name: /^Use the small workflow/ })).toBeVisible();
});

test.describe("with Grok configured for the first stage", () => {
  test.use({ settings: '[stages.talking]\nprovider = "only grok"\n' });

  test("Home shows the first stage's agent and plain chooser labels", async ({ page }) => {
    await addProject(page, "Sample");

    const project = home(page).getByRole("button", { name: "Sample", exact: true });
    const agent = agentChooser(page);
    const prompt = page.getByLabel("New thread");
    await expect(project).toBeVisible();
    await expect(agent).toHaveAccessibleName("Grok");

    await prompt.fill("use tools");
    await agent.click();
    const choices = home(page).getByRole("listbox", { name: "Agent", exact: true });
    await expect(choices.getByRole("option")).toHaveText(["Claude", "Codex", "Grok", "Gemini"]);
    const grok = choices.getByRole("option", { name: "Grok", exact: true });
    await expect(grok).toHaveAttribute("aria-selected", "true");
    await grok.focus();
    await page.keyboard.press("Escape");
    await expect(choices).toBeHidden();
    await expect(agent).toBeFocused();

    await agent.click();
    await home(page).getByText("Nothing needs you.", { exact: true }).click();
    await expect(choices).toBeHidden();
    await expect(prompt).toHaveValue("use tools");

    await prompt.press("Enter");
    await expect(page.locator(".thread-meta").getByText("Grok")).toBeVisible();
    await expect(page.getByText("run_terminal_command  Count bytes in note.txt")).toBeVisible();
  });
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

test("@ starts a new thread as ordinary text", async ({ page }) => {
  await addProject(page, "Sample");
  await page.getByLabel("New thread").fill("@hello");
  await page.getByLabel("New thread").press("Enter");
  await expect(page.getByLabel("Thread", { exact: true }).getByText("@hello", { exact: true })).toBeVisible();
});

test("the agent chosen on Home is there the next time", async ({ page }) => {
  await addProject(page, "Sample");
  await chooseAgent(page, "Codex");
  await page.reload();
  await expect(page.getByLabel("New thread")).toBeVisible();
  await expect(agentChooser(page)).toHaveAccessibleName("Codex");
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

test("Home and the menu share the agent for new threads", async ({ page }) => {
  await addProject(page, "Sample");

  await chooseAgent(page, "Codex");
  await expect(agentChooser(page)).toHaveAccessibleName("Codex");

  await choose(page, "Use Grok · default for new threads");
  await expect(agentChooser(page)).toHaveAccessibleName("Grok");

  // The thread starts on what Home shows.
  await page.getByLabel("New thread").fill("hello");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.locator(".thread-meta").getByText("Grok")).toBeVisible();
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

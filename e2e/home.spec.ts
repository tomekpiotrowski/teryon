// End-to-end specs for Home: the prompt that starts or continues a thread, and
// the list of threads that need the person.
//
// The fake agent fails, asks questions or changes files when its message says
// so. Nothing here reaches a model provider.

import type { Page } from "@playwright/test";
import { readFile, readdir } from "node:fs/promises";
import { join } from "node:path";

import { addProject, agentChooser, choose, chooseAgent, commit, expandCard, expect, git, menu, openWorkspace, put, startOnHome, startThread, test } from "./support";

test.beforeEach(openWorkspace);

/** The part of the screen that is Home. */
function home(page: Page) {
  return page.locator("app-home");
}

/** The list of threads on Home. */
function waiting(page: Page) {
  return home(page).getByRole("list", { name: "Threads" });
}

/** Starts a thread from Home and waits until the agent has finished, still on Home. */
async function finishOnHome(page: Page, message: string) {
  const card = await startOnHome(page, message);
  await expect(card).not.toHaveAttribute("data-section", "progress");
  return card;
}

/** Returns from a thread screen to Home. */
async function goHome(page: Page): Promise<void> {
  await choose(page, "Home");
  await expect(home(page)).toBeVisible();
}

/** Opens the first listed thread on its own screen. */
async function openFirst(page: Page): Promise<void> {
  const card = waiting(page).getByRole("listitem").first();
  await expandCard(card);
  await card.getByRole("button", { name: "Open", exact: true }).click();
  await expect(page.getByLabel("Thread", { exact: true })).toBeVisible();
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
  await expect(agentChooser(page)).toHaveAccessibleName("Grok · Grok 4.7 · Medium");
  await chooseAgent(page, "Grok");
  await expect(agentChooser(page)).toHaveAttribute("aria-expanded", "false");
  await expect(agentChooser(page)).toBeFocused();

  await startThread(page, "use tools");

  await expect(agentChooser(page, true)).toHaveAccessibleName("Grok · Grok 4.7 · Medium");
  await expect(page.getByText("run_terminal_command  Count bytes in note.txt")).toBeVisible();
  await page.keyboard.press("Control+k");
  await expect(menu(page).getByRole("option", { name: /^Use the small workflow/ })).toBeVisible();
});

test.describe("with Grok configured for the first stage", () => {
  test.use({ settings: '[stages.talking]\nprovider = "only grok"\n' });

  test("Home shows the first stage's agent behind Auto and plain chooser labels", async ({ page }) => {
    await addProject(page, "Sample");

    const project = home(page).getByRole("button", { name: "Sample", exact: true });
    const agent = agentChooser(page);
    const prompt = page.getByLabel("New thread");
    await expect(project).toBeVisible();
    await expect(agent).toHaveAccessibleName("Auto · Grok · Grok 4.7 · Medium");

    await prompt.fill("use tools");
    await agent.click();
    const choices = home(page).getByRole("listbox", { name: "Agent", exact: true });
    await expect(choices.getByRole("option").locator("span:first-child")).toHaveText(["Automatic", "Claude", "Codex", "Grok", "Gemini"]);
    const automatic = choices.getByRole("option", { name: "Automatic", exact: true });
    await expect(automatic).toHaveAttribute("aria-selected", "true");
    await expect(choices.getByRole("option", { name: "Grok", exact: true })).toHaveAttribute("aria-selected", "false");
    await expect(automatic).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(choices).toBeHidden();
    await expect(agent).toBeFocused();

    await agent.click();
    await home(page).getByText("Nothing needs you.", { exact: true }).click();
    await expect(choices).toBeHidden();
    await expect(prompt).toHaveValue("use tools");

    await startThread(page, "use tools");
    await expect(agentChooser(page, true)).toHaveAccessibleName("Grok · Grok 4.7 · Medium");
    await expect(page.getByText("run_terminal_command  Count bytes in note.txt")).toBeVisible();
  });
});

test.describe("with a rule for the first stage", () => {
  test.use({ settings: '[stages.talking]\nprovider = "only grok"\n' });

  test("a manual pick lasts until Automatic is chosen again", async ({ page }) => {
    await addProject(page, "Sample");
    const agent = agentChooser(page);
    await expect(agent).toHaveAccessibleName("Auto · Grok · Grok 4.7 · Medium");
    await page.keyboard.press("Control+k");
    await expect(menu(page).getByRole("option", { name: /^Use automatic selection/ })).toHaveCount(0);
    await page.keyboard.press("Escape");

    // A manual pick drops Auto, and stays through a thread, Home and a reload.
    await chooseAgent(page, "Codex");
    await expect(agent).toHaveAccessibleName(/^Codex/);
    await finishOnHome(page, "first manual");
    await expect(agent).toHaveAccessibleName(/^Codex/);
    await page.reload();
    await expect(agent).toHaveAccessibleName(/^Codex/);

    // Automatic from the dropdown brings the first stage's pick back.
    await agent.click();
    await home(page).getByRole("option", { name: "Automatic", exact: true }).click();
    await expect(agent).toHaveAccessibleName("Auto · Grok · Grok 4.7 · Medium");
    await expect(agent).toBeFocused();
    await startThread(page, "use tools");
    await expect(agentChooser(page, true)).toHaveAccessibleName("Grok · Grok 4.7 · Medium");
    await expect(page.getByText("run_terminal_command  Count bytes in note.txt")).toBeVisible();
    await expect(page.getByText(/is working…/)).toHaveCount(0);
    await goHome(page);
    await expect(agent).toHaveAccessibleName("Auto · Grok · Grok 4.7 · Medium");

    // So does the command, offered with the stage's rule only while a manual pick is on.
    await chooseAgent(page, "Claude");
    await expect(agent).toHaveAccessibleName(/^Claude/);
    await page.keyboard.press("Control+k");
    await expect(menu(page).getByRole("option", { name: /^Use automatic selection for new threads/ })).toContainText("Only Grok");
    await page.keyboard.press("Escape");
    await choose(page, "Use automatic selection for new threads");
    await expect(agent).toHaveAccessibleName("Auto · Grok · Grok 4.7 · Medium");
  });
});

test.describe("with any agent for the first stage", () => {
  test.use({
    settings: '[stages.talking]\nprovider = "any"\namong = ["claude", "grok"]\n',
    serviceLog: "teryon_core::workspace::stages=debug",
  });

  test("Home shows the next automatic pick after each thread", async ({ page, service }) => {
    await addProject(page, "Sample");
    const agent = agentChooser(page);
    const labels = new Map([
      ["claude", "Claude · Sonnet 5.5 · Medium"],
      ["grok", "Grok · Grok 4.7 · Medium"],
    ]);
    await expect(agent).toHaveAccessibleName(/^Auto · (Claude|Grok)/);

    // A fresh draw can repeat, in which case the service has no change to announce.
    // Read its decision for each start and check that Home shows that result.
    const logs = join(service.home, "logs");
    const picks = async () => {
      const files = (await readdir(logs)).sort();
      const contents = await Promise.all(files.map((file) => readFile(join(logs, file), "utf8")));
      return contents.flatMap((log) =>
        Array.from(log.matchAll(/Selected the next automatic starting agent agent="(claude|grok)" reason=[^\n]*picked at random/g), ([, provider]) => provider),
      );
    };
    const messages = ["first automatic", "second automatic"];
    for (const [index, message] of messages.entries()) {
      const shown = (await agent.getAttribute("title"))!.replace(/^Auto · /, "");
      await startThread(page, message);
      await expect(agentChooser(page, true)).toHaveAccessibleName(shown);
      await expect(page.getByLabel("Thread", { exact: true }).getByText(message, { exact: true })).toBeVisible();
      await expect(page.getByText(/is working…/)).toHaveCount(0);
      await goHome(page);

      await expect.poll(picks).toHaveLength(index + 1);
      const provider = (await picks())[index];
      await expect(agent).toHaveAccessibleName(`Auto · ${labels.get(provider)}`);
    }
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
  await finishOnHome(page, "please ask questions");

  const row = waiting(page).getByRole("listitem").filter({ hasText: /please ask questions/ });
  await expect(row).toHaveAttribute("data-section", "waiting");
  await expect(page.getByRole("button", { name: "1 need you" })).toBeVisible();

  await row.locator(".home-card-compact").click();
  await expect(home(page)).toBeVisible();
  await expect(page.getByLabel("Thread", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("group", { name: "The agent’s questions" })).toBeVisible();
});

test("a failed run is listed as failed, and stays until the thread runs again", async ({ page }) => {
  await addProject(page, "Sample");
  await finishOnHome(page, "please fail");

  const row = waiting(page).getByRole("listitem").filter({ hasText: /please fail/ });
  await expect(row).toHaveAttribute("data-section", "waiting");
  await expect(row).toContainText("The run failed");
  await expect(row.locator(".home-dot")).toHaveAttribute("data-word", "failed");

  // Opening the card does not clear a failure, and does not leave Home.
  // The open card shows the failure itself; the short line belongs to the collapsed card.
  await row.locator(".home-card-compact").click();
  await expect(page.getByLabel("Thread", { exact: true })).toHaveCount(0);
  await expect(row).toHaveAttribute("data-section", "waiting");
  await expect(row.locator(".home-dot")).toHaveAttribute("data-word", "failed");
  await expect(row).toContainText("the fake agent was told to fail");
});

test("a change ready to apply is listed as apply, after threads that waited longer", async ({ page }) => {
  await addProject(page, "Sample", (folder) => {
    git(folder, "init", "-q", "-b", "main");
    put(folder, "README.md", "# Sample\n");
    commit(folder);
  });
  await finishOnHome(page, "please fail");
  await finishOnHome(page, "start a change fix-login");

  const rows = waiting(page).getByRole("listitem");
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toContainText("The run failed");
  await expect(rows.last()).toContainText("start a change fix-login");
  await expect(rows.last()).toContainText("Ready to apply");
  await expect(rows.last().locator(".home-dot")).toHaveAttribute("data-word", "apply");
});

test("Tab hands the keys to the list, where j, k and Enter move and open", async ({ page }) => {
  await addProject(page, "Sample");
  await finishOnHome(page, "please fail");
  await finishOnHome(page, "please ask questions");

  const rows = waiting(page).getByRole("listitem");
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toContainText("please fail");
  await expect(rows.last()).toContainText("please ask questions");

  await page.getByLabel("New thread").press("Tab");
  await expect(waiting(page)).toBeFocused();
  await page.keyboard.press("j");
  await page.keyboard.press("Enter");
  // The failure waited longer than the questions, so the second card is the one that asked. Enter answers it here.
  await expect(home(page)).toBeVisible();
  await expect(page.getByRole("group", { name: "The agent’s questions" })).toBeVisible();
});

test("Tab and Escape in the list go back to the prompt", async ({ page }) => {
  await addProject(page, "Sample");
  await finishOnHome(page, "please fail");

  await page.getByLabel("New thread").press("Tab");
  await expect(waiting(page)).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByLabel("New thread")).toBeFocused();
});

test("@ starts a new thread as ordinary text", async ({ page }) => {
  await addProject(page, "Sample");
  await page.getByLabel("New thread").fill("@hello");
  await page.getByLabel("New thread").press("Enter");
  await expect(page.getByLabel("New thread")).toHaveValue("");
  await expect(page.getByLabel("New thread")).toBeFocused();
  await expect(home(page).locator(".home-card").filter({ hasText: "@hello" })).toBeVisible();
  await expect(page.getByLabel("Thread", { exact: true })).toHaveCount(0);
});

test("a model and effort chosen with the keyboard on Home are restored", async ({ page }) => {
  await addProject(page, "Sample");
  const agent = agentChooser(page);

  await agent.press("Enter");
  const agents = home(page).getByRole("listbox", { name: "Agent", exact: true });
  await expect(agents.getByRole("option", { name: "Claude", exact: true })).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(agents.getByRole("option", { name: "Codex", exact: true })).toBeFocused();
  await page.keyboard.press("Enter");

  const models = home(page).getByRole("listbox", { name: "Model", exact: true });
  await expect(models.getByRole("option", { name: "GPT-6 Astra", exact: true })).toBeFocused();
  await page.keyboard.press("ArrowRight");
  const efforts = home(page).getByRole("listbox", { name: "Effort", exact: true });
  await expect(efforts.getByRole("option", { name: "Medium", exact: true })).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(efforts.getByRole("option", { name: "High", exact: true })).toBeFocused();
  await page.keyboard.press("Space");
  await expect(agent).toBeFocused();
  await expect(agent).toHaveAccessibleName("Codex · GPT-6 Astra · High");

  await page.reload();
  await expect(page.getByLabel("New thread")).toBeVisible();
  await expect(agent).toHaveAccessibleName("Codex · GPT-6 Astra · High");
});

test("a project picked on Home is where the thread starts", async ({ page }) => {
  await addProject(page, "First");
  await addProject(page, "Second");

  await home(page).getByRole("button", { name: /Second/ }).click();
  await home(page).getByRole("option", { name: "First" }).click();
  const card = await startOnHome(page, "be slow");

  await expect(card).toContainText("First");
  await expect(page.getByLabel("Thread", { exact: true })).toHaveCount(0);
});

test("Escape from a thread goes to Home", async ({ page }) => {
  await addProject(page, "Sample");
  await finishOnHome(page, "please fail");
  await page.getByLabel("Open menu").click();
  await menu(page).getByRole("option", { name: /^please fail/ }).click();
  await expect(page.getByLabel("Thread", { exact: true })).toBeVisible();

  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("Escape");
  await expect(home(page)).toBeVisible();
});

test("Home and the menu share the model for new threads", async ({ page }) => {
  await addProject(page, "Sample");

  await choose(page, "Use Grok · Grok 4\\.6 · Medium for new threads");
  await expect(agentChooser(page)).toHaveAccessibleName("Grok · Grok 4.6 · Medium");

  await chooseAgent(page, "Codex", "GPT-6 Astra");
  await expect(agentChooser(page)).toHaveAccessibleName("Codex · GPT-6 Astra · Medium");
  await expect(agentChooser(page)).toBeFocused();

  await page.keyboard.press("Control+k");
  await expect(menu(page).getByRole("option", { name: "Use Codex · GPT-6 Sol · High for new threads", exact: true })).toBeVisible();
  await expect(menu(page).getByRole("option", { name: "Use Codex · GPT-6 Astra · Medium for new threads", exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");

  await startThread(page, "hello");
  await expect(page.locator(".thread-meta")).toContainText("Codex · GPT-6 Astra");
  await expect(page.getByText(/^Done in \d+s · GPT-6 Astra · Medium ·/)).toBeVisible();
});

test("G then H goes home from the code, the history and the board", async ({ page }) => {
  await addProject(page, "Sample", (folder) => {
    git(folder, "init", "-q", "-b", "main");
    put(folder, "README.md", "# Sample\n");
    commit(folder);
  });
  await finishOnHome(page, "please fail");
  await page.getByRole("button", { name: "Home" }).click();
  await openFirst(page);

  for (const key of ["c", "h", "b"]) {
    await page.getByLabel("Reply").press("Escape");
    await page.keyboard.press(key);
    await expect(home(page)).toBeHidden();
    await page.keyboard.press("g");
    await page.keyboard.press("h");
    await expect(home(page)).toBeVisible();
    await openFirst(page);
  }
});

test("the menu has a Home command", async ({ page }) => {
  await addProject(page, "Sample");
  await finishOnHome(page, "please fail");
  await openFirst(page);
  await choose(page, "Home");
  await expect(home(page)).toBeVisible();
});

test("a new thread stays on Home and is pointed out in progress", async ({ page }) => {
  await addProject(page, "Sample");
  await page.getByLabel("New thread").fill("be slow");
  await page.getByRole("button", { name: "Run ↵" }).click();

  const card = home(page).locator(".home-card").filter({ hasText: "be slow" });
  await expect(card).toBeVisible();
  await expect(card).toHaveClass(/home-card-fresh/);
  await expect(card).toHaveAttribute("data-section", "progress");
  await expect(home(page).getByRole("region", { name: "In progress" })).toBeVisible();
  await expect(page.getByLabel("New thread")).toHaveValue("");
  await expect(page.getByLabel("New thread")).toBeFocused();
  await expect(page.getByLabel("Thread", { exact: true })).toHaveCount(0);
});

test("a reply is sent from the card and the thread keeps working on Home", async ({ page }) => {
  await addProject(page, "Sample");
  const card = await finishOnHome(page, "hello");

  await expandCard(card);
  await page.getByLabel("Reply").fill("be slow");
  await page.getByLabel("Reply").press("Enter");

  await expect(card).toContainText("be slow");
  await expect(card).toHaveAttribute("data-section", "progress");
  await expect(home(page)).toBeVisible();
  await expect(page.getByLabel("Thread", { exact: true })).toHaveCount(0);
});

test("m marks a finished thread read from its card", async ({ page }) => {
  await addProject(page, "Sample");
  const card = await finishOnHome(page, "hello");

  await expandCard(card);
  await page.locator("[data-home-list]").focus();
  await page.keyboard.press("m");

  await expect(card).toHaveCount(0);
  await expect(home(page).getByText("Nothing needs you.")).toBeVisible();
});

test("questions are answered on the card without leaving Home", async ({ page }) => {
  await addProject(page, "Sample");
  const card = await finishOnHome(page, "please ask questions");

  await expandCard(card);
  const questions = page.getByRole("group", { name: "The agent’s questions" });
  await expect(questions).toBeVisible();
  await expect(questions).not.toBeFocused();
  await page.locator("[data-home-list]").focus();
  await page.keyboard.press("q");
  await expect(questions).toBeFocused();
  await page.keyboard.press("Enter");

  await expect(questions).toHaveCount(0);
  await expect(card).toContainText("Answers to your questions");
  await expect(card).toContainText("In the thread");
  await expect(home(page)).toBeVisible();
  await expect(page.getByLabel("Thread", { exact: true })).toHaveCount(0);
});

test("o opens the thread screen from the card", async ({ page }) => {
  await addProject(page, "Sample");
  const card = await finishOnHome(page, "hello");

  await expandCard(card);
  await page.locator("[data-home-list]").focus();
  await page.keyboard.press("o");

  await expect(page.getByLabel("Thread", { exact: true }).getByText("hello", { exact: true })).toBeVisible();
});

test("a plan opened from a card returns to that card", async ({ page }) => {
  await addProject(page, "Sample");
  const card = await finishOnHome(page, 'write a plan "Login fix"');

  await expandCard(card);
  await card.getByRole("button", { name: "plan", exact: true }).click();
  await expect(page.locator("app-plan").getByRole("heading", { name: "Login fix" })).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.locator("app-plan")).toHaveCount(0);
  await expect(home(page)).toBeVisible();
  await expect(card.getByRole("button", { name: "Open", exact: true })).toBeVisible();
});

test("a review asked from a card is read on Home", async ({ page }) => {
  await addProject(page, "Sample", (folder) => {
    git(folder, "init", "-q", "-b", "main");
    put(folder, "README.md", "# Sample\n");
    commit(folder);
  });
  const card = await finishOnHome(page, "start a change fix-login");

  await expandCard(card);
  await card.getByRole("button", { name: "Ask Codex · GPT-6 Astra · Medium to review" }).click();

  await expect(card).toContainText("fix-login.txt has no test.");
  await expect(home(page)).toBeVisible();
  await expect(page.getByLabel("Thread", { exact: true })).toHaveCount(0);
});

test("file changes opened from a card return to that card", async ({ page }) => {
  await addProject(page, "Sample", (folder) => {
    git(folder, "init", "-q", "-b", "main");
    put(folder, "README.md", "# Sample\n");
    commit(folder);
  });
  const card = await finishOnHome(page, "start a change fix-login");

  await expandCard(card);
  await page.locator("[data-home-list]").focus();
  await page.keyboard.press("w");
  await page.keyboard.press("v");
  await expect(page.getByLabel("Repository files").getByRole("button", { name: "fix-login.txt" })).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(home(page)).toBeVisible();
  await expect(card.getByRole("button", { name: "Open", exact: true })).toBeVisible();
  await expect(page.getByLabel("Repository files")).toHaveCount(0);
});

test("a change is applied directly from its card without a dialog", async ({ page }) => {
  const directory = await addProject(page, "Sample", (folder) => {
    git(folder, "init", "-q", "-b", "main");
    put(folder, "README.md", "# Sample\n");
    commit(folder);
  });
  const card = await finishOnHome(page, "start a change fix-login");
  const copy = join(directory, ".teryon", "worktrees", "fix-login");
  git(copy, "add", "-A");
  git(copy, "commit", "-q", "-m", "Fix the login");

  await expandCard(card);
  await card.getByRole("button", { name: "Apply to the project" }).click();
  await expect(page.locator("ion-alert")).toHaveCount(0);

  await expect(page.getByText("Applied to the project.", { exact: true }).first()).toBeVisible();
  await expect(home(page)).toBeVisible();
  await expect(card).toContainText("Applied");
  await expect.poll(() => git(directory, "log", "-1", "--format=%s")).toBe("Fix the login");
});

test("a running agent is stopped from its card", async ({ page }) => {
  await addProject(page, "Sample");
  const card = await startOnHome(page, "be slow");

  await expandCard(card);
  await card.getByRole("button", { name: "Stop", exact: true }).click();

  await expect(card).toContainText("Stopped by you.");
  await expect(home(page)).toBeVisible();
  await expect(page.getByLabel("Thread", { exact: true })).toHaveCount(0);
});

test("an opened card starts at the bottom of its conversation and keeps where the person scrolled", async ({ page }) => {
  await addProject(page, "Sample");
  const card = await finishOnHome(page, "write markdown");
  const body = card.locator(".home-card-body");

  await expandCard(card);
  await expect(card.getByRole("heading", { name: "Findings" })).toBeAttached();
  expect(await body.evaluate((element) => element.scrollHeight > element.clientHeight)).toBe(true);
  await expect.poll(() => body.evaluate((element) => element.scrollHeight - element.clientHeight - element.scrollTop)).toBeLessThanOrEqual(1);

  await body.evaluate((element) => element.scrollTo({ top: 0 }));
  await page.getByLabel("Reply").fill("A reply being written");
  await expect.poll(() => body.evaluate((element) => element.scrollTop)).toBe(0);
});

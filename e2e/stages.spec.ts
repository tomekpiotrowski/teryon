// End-to-end specs for thread stages: an agent reports where its work
// stands, the next stage's agent starts by itself, and the board shows every
// thread by stage.
//
// The fake agent runs the real `teryon thread stage` when its message says
// "report stage <stage>". Nothing here reaches a model provider.

import type { Page } from "@playwright/test";

import { startThread, addProject, agentChooser, choose, commit, expect, git, menu, openWorkspace, put, test } from "./support";

test.beforeEach(openWorkspace);

test.describe("with an agent set for implementing", () => {
  test.use({ settings: '[stages.implementing]\nprovider = "only claude"\nstrength = "fast"\n' });

  test("the next stage's agent starts by itself on its own model", async ({ page }) => {
    await addProject(page, "Sample");

    await page.getByRole("button", { name: "Settings", exact: true }).click();
    const claude = page.getByRole("group", { name: "Claude", exact: true });
    await claude.getByLabel("Fast effort").selectOption("high");
    await page.getByRole("button", { name: "Save", exact: true }).click();
    await expect(page.getByRole("button", { name: "Save", exact: true })).toBeDisabled();
    await page.keyboard.press("Escape");
    await expect(page.locator("app-settings")).toHaveCount(0);

    await startThread(page, "report stage implementing");

    const thread = page.getByLabel("Thread");
    await expect(thread.getByText("◇ Talking → Implementing · by Claude")).toBeVisible();
    await expect(thread.getByText("Claude · haiku · High takes Implementing (only Claude).")).toBeVisible();
    await expect(thread.getByText("Implement what was discussed above.")).toBeVisible();
    await expect(agentChooser(page, true)).toHaveAccessibleName("Claude · Haiku 5.5 · High");
    await expect(page.locator(".thread-stage")).toContainText("Implementing");
  });
});

test("the board shows each thread in the column of its stage, and follows it", async ({ page }) => {
  await addProject(page, "Sample");
  await startThread(page, "report stage planning");
  await expect(page.getByLabel("Thread").getByText("◇ Talking → Planning · by Claude")).toBeVisible();

  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("b");
  const board = page.locator("app-board");
  await expect(board.getByRole("region", { name: "Talking" })).toBeVisible();
  await expect(board.getByRole("region", { name: "Planning" }).getByRole("button", { name: /report stage planning/ })).toBeVisible();

  // `h` and `l` move between columns here, rather than opening the history.
  await page.keyboard.press("h");
  await expect(board.locator(".board-card.at")).toHaveCount(0);
  await expect(page.locator("app-history")).toHaveCount(0);
  await page.keyboard.press("l");
  await expect(board.locator(".board-card.at")).toContainText("report stage planning");

  await page.keyboard.press("Enter");
  await expect(board).toHaveCount(0);
  await page.getByLabel("Reply").fill("report stage implementing");
  await page.getByRole("button", { name: "send ↵" }).click();
  await expect(page.getByLabel("Thread").getByText("◇ Planning → Implementing · by Claude")).toBeVisible();

  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("b");
  await expect(board.getByRole("region", { name: "Implementing" }).getByRole("button", { name: /report stage planning/ })).toBeVisible();
  await expect(board.getByRole("region", { name: "Planning" }).getByRole("button")).toHaveCount(0);

  await page.keyboard.press("Escape");
  await expect(board).toHaveCount(0);
});

/** The menu section a command is listed under. The menu is closed again afterwards. */
async function sectionOf(page: Page, title: string): Promise<string> {
  await page.keyboard.press("Control+k");
  const list = page.locator("#menu-list");
  await list.waitFor();
  const section = await list.evaluate((node, wanted) => {
    let heading = "";
    for (const child of node.children) {
      if (child.classList.contains("menu-heading")) heading = child.textContent ?? "";
      if (child.getAttribute("role") === "option" && (child.textContent ?? "").includes(wanted)) return heading;
    }
    return "";
  }, title);
  await page.keyboard.press("Escape");
  await expect(menu(page)).toHaveCount(0);
  return section;
}

test("a finish stays for the visit where it is seen, then leaves the board", async ({ page }) => {
  await addProject(page, "Sample", (folder) => {
    git(folder, "init", "-q", "-b", "main");
    put(folder, "README.md", "# Sample\n");
    commit(folder);
  });
  await startThread(page, "start a change fix-login");
  await expect(page.getByRole("region", { name: "This thread’s changes" })).toContainText("1 file changed");
  await expect(page.getByText(/^Done in/)).toBeVisible();

  await choose(page, "Discard changes");
  await page.locator("ion-alert").getByRole("button", { name: "Discard" }).click();
  await expect(page.locator("ion-alert")).toHaveCount(0);
  await expect(page.locator(".thread-stage")).toContainText("Done");

  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("b");
  const board = page.locator("app-board");
  await expect(board.getByRole("region", { name: "Done" }).getByRole("button", { name: /start a change fix-login/ })).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(board).toHaveCount(0);
  await page.keyboard.press("b");
  await expect(board.getByRole("region", { name: "Done" })).toHaveCount(0);

  await page.keyboard.press("Escape");
  await expect.poll(() => sectionOf(page, "start a change fix-login")).toBe("Threads");
  await expect(menu(page)).toHaveCount(0);
});

test("quiet threads leave together, and writing brings one back", async ({ page }) => {
  await addProject(page, "Sample", (folder) => {
    git(folder, "init", "-q", "-b", "main");
    put(folder, "README.md", "# Sample\n");
    commit(folder);
  });

  await startThread(page, "hello there");
  await expect(page.getByText(/^Done in/)).toBeVisible();

  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("n");
  await startThread(page, "start a change fix-login");
  await expect(page.getByRole("region", { name: "This thread’s changes" })).toContainText("1 file changed");
  await expect(page.getByText(/^Done in/)).toBeVisible();

  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("b");
  const board = page.locator("app-board");
  const talking = board.getByRole("region", { name: "Talking" });
  await expect(talking.getByRole("button", { name: /hello there/ })).toBeVisible();
  await expect(talking.getByRole("button", { name: /start a change fix-login/ })).toBeVisible();

  await page.keyboard.press("x");
  const alert = page.locator("ion-alert");
  await expect(alert).toContainText("Put away 1 thread in Sample?");
  await alert.getByRole("button", { name: "Cancel" }).click();
  await expect(alert).toHaveCount(0);
  await expect(talking.getByRole("button", { name: /hello there/ })).toBeVisible();
  await expect(talking.getByRole("button", { name: /start a change fix-login/ })).toBeVisible();

  await page.keyboard.press("x");
  await expect(alert).toContainText("They leave the board and stay in the menu.");
  await alert.getByRole("button", { name: "Put away" }).click();
  await expect(alert).toHaveCount(0);
  await expect(talking.getByRole("button", { name: /hello there/ })).toHaveCount(0);
  await expect(talking.getByRole("button", { name: /start a change fix-login/ })).toBeVisible();

  await page.keyboard.press("Escape");
  await expect.poll(() => sectionOf(page, "hello there")).toBe("Put away");
  await choose(page, "hello there");
  await expect(page.locator(".thread-stage")).toContainText("Talking · put away");

  await page.getByLabel("Reply").fill("back again");
  await page.getByRole("button", { name: "send ↵" }).click();
  await expect(page.locator(".thread-stage")).not.toContainText("put away");

  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("b");
  await expect(board.getByRole("region", { name: "Talking" }).getByRole("button", { name: /hello there/ })).toBeVisible();
});

test("the agent's plan shows as a line, opens on P or the plan button, and follows changes", async ({ page }) => {
  await addProject(page, "Sample");
  await startThread(page, 'write a plan "Login fix"');

  const thread = page.getByLabel("Thread");
  await expect(thread.getByText("◇ Talking → Planning · a plan was written")).toBeVisible();
  const planLine = thread.getByRole("button", { name: "Plan written · Login fix" });
  await expect(planLine).toBeVisible();
  // The line wears the accent colour, not the grey of the notices around it.
  const colourOf = (line: typeof planLine) => line.evaluate((element) => getComputedStyle(element).color);
  expect(await colourOf(planLine)).not.toBe(await thread.locator(".stage-line").first().evaluate((element) => getComputedStyle(element).color));

  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("p");
  const plan = page.locator("app-plan");
  await expect(plan.getByText("plan · write a plan")).toBeVisible();
  await expect(plan.getByRole("heading", { name: "Login fix" })).toBeVisible();
  await expect(plan.getByText("Steps for Login fix.")).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(plan).toHaveCount(0);
  const planButton = page.locator(".thread-composer").getByRole("button", { name: "plan", exact: true });
  await planButton.click();
  await expect(plan.getByRole("heading", { name: "Login fix" })).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(plan).toHaveCount(0);
  await page.getByLabel("Reply").fill('write a plan "Login fix, take two"');
  await page.getByRole("button", { name: "send ↵" }).click();
  await thread.getByRole("button", { name: "Plan changed · Login fix, take two" }).click();
  await expect(plan.getByRole("heading", { name: "Login fix, take two" })).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(planButton).toBeVisible();
});

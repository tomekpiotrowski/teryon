// End-to-end specs for thread stages: an agent reports where its work
// stands, the next stage's agent starts by itself, and the board shows every
// thread by stage.
//
// The fake agent runs the real `teryon thread stage` when its message says
// "report stage <stage>". Nothing here reaches a model provider.

import { addProject, expect, openWorkspace, test } from "./support";

test.beforeEach(openWorkspace);

test.describe("with an agent set for implementing", () => {
  test.use({ settings: '[stages.implementing]\nprovider = "only claude"\nstrength = "fast"\n' });

  test("the next stage's agent starts by itself on its own model", async ({ page }) => {
    await addProject(page, "Sample");

    await page.getByLabel("New thread").fill("report stage implementing");
    await page.getByRole("button", { name: "Run ↵" }).click();

    const thread = page.getByLabel("Thread");
    await expect(thread.getByText("◇ Talking → Implementing · by Claude")).toBeVisible();
    await expect(thread.getByText("Claude · haiku takes Implementing (only Claude).")).toBeVisible();
    await expect(thread.getByText("Implement what was discussed above.")).toBeVisible();
    await expect(page.locator(".thread-choice")).toHaveText("Claude · Haiku");
    await expect(page.locator(".thread-stage")).toContainText("Implementing");
  });
});

test("the board shows each thread in the column of its stage, and follows it", async ({ page }) => {
  await addProject(page, "Sample");
  await page.getByLabel("New thread").fill("report stage planning");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.getByLabel("Thread").getByText("◇ Talking → Planning · by Claude")).toBeVisible();

  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("b");
  const board = page.locator("app-board");
  await expect(board.getByText("board · Sample")).toBeVisible();
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

test("the agent's plan shows as a line, opens on P or the plan button, and follows changes", async ({ page }) => {
  await addProject(page, "Sample");
  await page.getByLabel("New thread").fill('write a plan "Login fix"');
  await page.getByRole("button", { name: "Run ↵" }).click();

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

// End-to-end spec for the settings screen: everyday choices and the workflows
// themselves, saved on this machine and read by the next decision.
//
// A turn already going is left alone. What these tests can see is the next
// look at the screen: the stage button, the menu, and the form after it
// is opened again.

import type { Page } from "@playwright/test";

import { startThread, addProject, commit, expect, git, menu, openWorkspace, put, test } from "./support";

test.beforeEach(openWorkspace);

/** Opens Settings from the workspace toolbar. */
async function openSettings(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.locator("app-settings")).toBeVisible();
}

test("saved choices come back, and the thread's stage button uses the new label", async ({ page }) => {
  await addProject(page, "Sample");
  await startThread(page, "hello");
  await expect(page.getByText(/^Done in/)).toBeVisible();
  await expect(page.locator(".thread-stage")).toContainText("Talking");

  await page.getByLabel("Reply").press("Escape");
  await openSettings(page);
  const settings = page.locator("app-settings");

  const talking = settings.getByLabel("Label for Talking");
  await talking.fill("Chatting");
  await settings.getByRole("button", { name: "Discard" }).click();
  await expect(talking).toHaveValue("");

  await settings.getByLabel("New threads follow").selectOption({ label: "small" });
  await settings.getByLabel("Agents started on their own").fill("1");
  await settings.getByRole("group", { name: "small" }).getByLabel("Description").fill("Short changes.");
  await talking.fill("Chatting");

  const implementing = settings.getByRole("group", { name: "Implementing" });
  await implementing.getByLabel("Who works").selectOption("only");
  await implementing.getByLabel("Agent").selectOption("grok");
  await implementing.getByLabel("Strength").selectOption("fast");

  await settings.getByRole("group", { name: "Claude" }).getByLabel("Strong", { exact: true }).fill("opus-test");
  await settings.getByRole("group", { name: "Grok" }).getByLabel("Fast", { exact: true }).fill("grok-test");
  await settings.getByRole("group", { name: "Grok" }).getByLabel("Fast effort").selectOption({ label: "Extra high" });
  await settings.getByLabel("Editor").fill("code {path}");
  await settings.getByLabel("Terminal").fill("alacritty --working-directory {path}");

  await settings.getByRole("button", { name: "Save", exact: true }).click();
  await expect(settings.getByRole("button", { name: "Save", exact: true })).toBeDisabled();

  await page.keyboard.press("Escape");
  await expect(settings).toHaveCount(0);
  await expect(page.locator(".thread-stage")).toContainText("Chatting");

  await openSettings(page);
  await expect(settings.getByLabel("New threads follow")).toHaveValue("small");
  await expect(settings.getByLabel("Agents started on their own")).toHaveValue("1");
  await expect(settings.getByRole("group", { name: "small" }).getByLabel("Description")).toHaveValue("Short changes.");
  await expect(settings.getByLabel("Label for Talking")).toHaveValue("Chatting");
  await expect(implementing.getByLabel("Who works")).toHaveValue("only");
  await expect(implementing.getByLabel("Agent")).toHaveValue("grok");
  await expect(implementing.getByLabel("Strength")).toHaveValue("fast");
  await expect(settings.getByRole("group", { name: "Claude" }).getByLabel("Strong", { exact: true })).toHaveValue("opus-test");
  await expect(settings.getByRole("group", { name: "Grok" }).getByLabel("Fast", { exact: true })).toHaveValue("grok-test");
  await expect(settings.getByRole("group", { name: "Grok" }).getByLabel("Fast effort")).toHaveValue("xhigh");
  await expect(settings.getByLabel("Editor")).toHaveValue("code {path}");
  await expect(settings.getByLabel("Terminal")).toHaveValue("alacritty --working-directory {path}");
});

test("a workflow that starts at review stays unsaved, and a later save is offered", async ({ page }) => {
  await openSettings(page);
  const settings = page.locator("app-settings");

  await settings.getByRole("button", { name: "Add a workflow" }).click();
  const stages = settings.getByRole("group", { name: "Stages of the new workflow" });
  await stages.getByRole("checkbox", { name: "Talking" }).uncheck();
  await stages.getByRole("checkbox", { name: "Planning" }).uncheck();
  await stages.getByRole("checkbox", { name: "Implementing" }).uncheck();
  await settings.getByRole("group", { name: "New workflow" }).getByLabel("Name").fill("late");

  await settings.getByRole("button", { name: "Save", exact: true }).click();
  await expect(settings.getByRole("alert")).toContainText("has to start with talking");
  await expect(settings.getByRole("group", { name: "late" }).getByLabel("Name")).toBeEnabled();

  await settings.getByRole("group", { name: "Stages of late" }).getByRole("checkbox", { name: "Talking" }).check();
  await settings.getByRole("button", { name: "Save", exact: true }).click();
  await expect(settings.getByRole("alert")).toHaveCount(0);
  await expect(settings.getByRole("group", { name: "late" }).getByLabel("Name")).toBeDisabled();

  await page.keyboard.press("Escape");
  await addProject(page, "Sample");
  await startThread(page, "hello");
  await expect(page.getByText(/^Done in/)).toBeVisible();
  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("Control+k");
  await expect(menu(page).getByRole("option", { name: /^Use the late workflow/ })).toBeVisible();
});

test("the editor and the terminal show in the menu once they are set, and leave once cleared", async ({ page }) => {
  await addProject(page, "Sample", (folder) => {
    git(folder, "init", "-q", "-b", "main");
    put(folder, "README.md", "# Sample\n");
    commit(folder);
  });
  await startThread(page, "start a change fix-login");
  await expect(page.getByLabel("Thread").getByText("ready", { exact: true })).toBeVisible();

  await page.getByLabel("Reply").press("Escape");
  await openSettings(page);
  const settings = page.locator("app-settings");
  await settings.getByLabel("Editor").fill("code {path}");
  await settings.getByLabel("Terminal").fill("alacritty --working-directory {path}");
  await settings.getByRole("button", { name: "Save", exact: true }).click();
  await expect(settings.getByRole("button", { name: "Save", exact: true })).toBeDisabled();

  await page.keyboard.press("Escape");
  await page.keyboard.press("Control+k");
  await expect(menu(page).getByRole("option", { name: /^Open in editor/ })).toBeVisible();
  await expect(menu(page).getByRole("option", { name: /^Open terminal/ })).toBeVisible();
  await page.keyboard.press("Escape");

  await openSettings(page);
  await settings.getByLabel("Editor").fill("");
  await settings.getByLabel("Terminal").fill("");
  await settings.getByRole("button", { name: "Save", exact: true }).click();
  await expect(settings.getByRole("button", { name: "Save", exact: true })).toBeDisabled();

  await page.keyboard.press("Escape");
  await page.keyboard.press("Control+k");
  await expect(menu(page).getByRole("option", { name: /^Open in editor/ })).toHaveCount(0);
  await expect(menu(page).getByRole("option", { name: /^Open terminal/ })).toHaveCount(0);
});

test("the settings screen fits a phone", async ({ page }) => {
  await openSettings(page);
  await page.setViewportSize({ width: 390, height: 844 });
  const settings = page.locator("app-settings");
  await expect(settings.getByRole("heading", { name: "Opening a copy" })).toBeVisible();
  const save = settings.getByRole("button", { name: "Save", exact: true });
  await save.scrollIntoViewIfNeeded();
  await expect(save).toBeVisible();
  const overflow = await settings.locator(".settings-body").evaluate((element) => element.scrollWidth - element.clientWidth);
  expect(overflow).toBeLessThanOrEqual(1);
});

// End-to-end specs for the workspace: a person adds a project and
// has an agent work in it.
//
// Each test has a Teryon service of its own, which starts a script that
// replays recordings in place of the agent programs. Nothing here reaches a model provider.

import { realpathSync } from "node:fs";
import { join } from "node:path";

import { addProject, choose, commit, expect, git, menu, openWorkspace, put, test } from "./support";

test.beforeEach(openWorkspace);

test("an agent works in a project and the thread fills in live", async ({ page }) => {
  await addProject(page, "Sample");

  await page.getByLabel("New thread").fill("hello");
  await page.getByRole("button", { name: "Run ↵" }).click();

  await expect(page.getByLabel("Thread").getByText("hello", { exact: true })).toBeVisible();
  await expect(page.getByLabel("Thread").getByText("ready", { exact: true })).toBeVisible();
  await expect(page.getByText(/^Done in \d+s · \$0\.21$/)).toBeVisible();
  await expect(page.locator(".workspace-bar")).toContainText("Sample · hello");

  await page.getByLabel("Reply").fill("use tools");
  await page.getByRole("button", { name: "send ↵" }).click();

  await expect(page.getByText("Bash  Check repository state and existing worktrees")).toBeVisible();
  await expect(page.getByLabel("Thread").locator("code", { hasText: /^note\.txt$/ })).toBeVisible();
  await expect(page.getByText(/is 6 bytes/)).toBeVisible();
  await expect(page.getByLabel("Reply")).toHaveValue("");
});

test("an agent's change shows where it stands, and its copy can be found", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const directory = await addProject(page, "Sample", (folder) => {
    git(folder, "init", "-q", "-b", "main");
    put(folder, "README.md", "# Sample\n");
    commit(folder);
  });

  await page.getByLabel("New thread").fill("start a change fix-login");
  await page.getByRole("button", { name: "Run ↵" }).click();

  const thread = page.getByLabel("Thread");
  await expect(thread.getByText("ready", { exact: true })).toBeVisible();
  const changes = page.getByLabel("Changes");
  await expect(changes).toContainText("fix-login");
  await expect(changes).toContainText("· 1 file changed");
  await expect(changes).toContainText("· ready to apply");
  await expect(page.getByText(/\.teryon\/worktrees/)).toHaveCount(0);

  // W, then Y, copies where the thread's copy is, saying so in the bar while waiting for the second key.
  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("w");
  await expect(page.locator(".bar-leader")).toHaveText("W …");
  await page.keyboard.press("y");
  await expect(page.locator(".bar-leader")).toHaveCount(0);
  await expect(page.getByText("Location copied.")).toBeVisible();
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  expect(copied).toBe(join(realpathSync(directory), ".teryon", "worktrees", "fix-login"));

  // The menu lists the same, with its keys.
  await page.keyboard.press("Control+k");
  await expect(menu(page).getByRole("option", { name: /^Copy location fix-login W Y$/ })).toBeVisible();
  await expect(menu(page).getByRole("option", { name: /^Open folder fix-login W F$/ })).toBeVisible();
  await expect(menu(page).getByRole("option", { name: /^Open in editor/ })).toHaveCount(0);
  await page.keyboard.press("Escape");

  await page.getByRole("button", { name: "Explore fix-login" }).click();
  await expect(page.getByLabel("Repository files").getByRole("button", { name: "fix-login.txt" })).toBeVisible();
  await expect(page.getByText(/\.teryon\/worktrees/)).toHaveCount(0);
});

test("an agent's markdown is rendered, and what it must not act on is shown as text", async ({ page }) => {
  await addProject(page, "Sample");

  await page.getByLabel("New thread").fill("write markdown");
  await page.getByRole("button", { name: "Run ↵" }).click();

  const thread = page.getByLabel("Thread");
  await expect(thread.getByRole("heading", { name: "Findings" })).toBeVisible();
  await expect(thread.locator("strong", { hasText: "parser" })).toBeVisible();
  await expect(thread.getByRole("listitem")).toHaveText(["Open src/parser.rs", "Run the tests"]);
  await expect(thread.locator("li code")).toHaveText("src/parser.rs");
  await expect(thread.locator("pre code")).toHaveText('fn main() {\n    println!("<hello>");\n}\n');

  // A column's alignment survives Angular's cleaning of the HTML.
  await expect(thread.getByRole("columnheader", { name: "Lines" })).toBeVisible();
  await expect(thread.getByRole("cell", { name: "120" })).toHaveCSS("text-align", "right");
  await expect(thread.getByRole("cell", { name: "parser.rs" })).toHaveCSS("text-align", "left");

  // A web address can be followed, in a new tab. One inside the project cannot.
  const report = thread.getByRole("link", { name: "the report" });
  await expect(report).toHaveAttribute("href", "https://example.com/report");
  await expect(report).toHaveAttribute("target", "_blank");
  await expect(report).toHaveAttribute("rel", "noopener noreferrer");
  await expect(thread.getByRole("link", { name: "the parser" })).toHaveCount(0);
  await expect(thread.getByText("the parser", { exact: true })).toHaveAttribute("title", "src/parser.rs");

  // HTML the agent wrote is shown, not run, and a picture is only a link.
  await expect(thread.getByText(/<img src="https:\/\/example\.com\/pixel\.png"/)).toBeVisible();
  await expect(thread.getByRole("link", { name: "A chart" })).toHaveAttribute("href", "https://example.com/chart.png");
  await expect(thread.locator("img")).toHaveCount(0);
  await expect(page).not.toHaveTitle("compromised");

  // What the person typed is never treated as markdown.
  await page.getByLabel("Reply").fill("**keep** the `marks`");
  await page.getByRole("button", { name: "send ↵" }).click();
  await expect(thread.getByText("**keep** the `marks`", { exact: true })).toBeVisible();
});

test("Grok's markdown is rendered too", async ({ page }) => {
  await addProject(page, "Sample");

  await page.getByRole("button", { name: "Change agent" }).click();
  await page.getByPlaceholder("What should Grok do in Sample?").fill("write markdown");
  await page.getByRole("button", { name: "Run ↵" }).click();

  const thread = page.getByLabel("Thread");
  await expect(thread.getByRole("heading", { name: "Findings" })).toBeVisible();
  await expect(thread.locator("pre code")).toContainText('println!("<hello>");');
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

test("a refused project says why, and can be corrected", async ({ page }) => {
  await choose(page, "New project");
  await page.getByLabel("Project name").fill("Nowhere");
  await page.getByLabel("Project directory").fill("/a/directory/nobody/has");
  await page.getByRole("button", { name: "Add project" }).click();

  await expect(page.getByText("The directory does not exist.")).toBeVisible();
  await expect(page.getByLabel("Project directory")).toHaveValue("/a/directory/nobody/has");
  await expect(page.getByText("Add a project to get started.")).toBeVisible();

  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(menu(page)).toHaveCount(0);
});

test("a project can be removed", async ({ page }) => {
  await addProject(page, "Sample");

  await choose(page, "Remove project Sample");
  await page.getByRole("button", { name: "Remove", exact: true }).click();

  await expect(page.getByText("Add a project to get started.")).toBeVisible();
});

test("a slash opens the menu, except where it is typed as text", async ({ page }) => {
  await addProject(page, "Sample");
  const box = page.getByLabel("New thread");

  // In the empty message box, as the agents' own programs take it.
  await box.focus();
  await page.keyboard.press("/");
  await expect(menu(page)).toBeVisible();
  await expect(page.getByLabel("Search the menu")).toBeFocused();
  await expect(page.getByLabel("Search the menu")).toHaveValue("");

  // Escape closes it and goes back to the message box.
  await page.keyboard.press("Escape");
  await expect(menu(page)).toHaveCount(0);
  await expect(box).toBeFocused();
  await expect(box).toHaveValue("");

  // After some text it is a character, so a path can be typed.
  await page.keyboard.type("look at src/main");
  await expect(menu(page)).toHaveCount(0);
  await expect(box).toHaveValue("look at src/main");

  // Ctrl+K opens it all the same, and closes it again, and the text is kept.
  await page.keyboard.press("Control+k");
  await expect(menu(page)).toBeVisible();
  await page.keyboard.press("Control+k");
  await expect(menu(page)).toHaveCount(0);
  await expect(box).toHaveValue("look at src/main");

  // Away from any text it opens the menu, and a click beside the menu closes it.
  await box.press("Escape");
  await page.keyboard.press("/");
  await expect(menu(page)).toBeVisible();
  await page.mouse.click(5, 5);
  await expect(menu(page)).toHaveCount(0);

  // The button on screen does the same.
  await page.getByRole("button", { name: "menu /" }).click();
  await expect(menu(page)).toBeVisible();
});

test("the menu lists threads, projects and actions, and typing narrows them", async ({ page }) => {
  await addProject(page, "Sample");
  await page.getByLabel("New thread").fill("hello");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.getByText(/^Done in/)).toBeVisible();
  await addProject(page, "Other");

  await page.keyboard.press("Control+k");
  const options = menu(page).getByRole("option");
  await expect(menu(page).locator(".menu-heading")).toHaveText(["Threads", "Projects", "Actions", "Agent commands"]);
  await expect(options.first()).toHaveText(/hello\s*Sample/);
  await expect(options.first()).toHaveAttribute("aria-selected", "true");
  await expect(menu(page).getByRole("option", { name: /^Files/ })).toContainText("C");

  // Typing narrows the list, and the arrows move through what is left, going round at the end.
  await expect(page.getByLabel("Search the menu")).toBeFocused();
  await page.keyboard.type("sam");
  await expect(options).toHaveText([/^\W*Sample/, /hello\s*Sample/]);
  await expect(menu(page).locator(".menu-heading")).toHaveCount(0);
  await page.keyboard.press("ArrowDown");
  await expect(options.nth(1)).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("ArrowDown");
  await expect(options.first()).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("ArrowUp");
  await expect(options.nth(1)).toHaveAttribute("aria-selected", "true");

  // Enter opens the marked thread, ready for a reply.
  await page.keyboard.press("Enter");
  await expect(menu(page)).toHaveCount(0);
  await expect(page.locator(".workspace-bar")).toContainText("Sample · hello");
  await expect(page.getByLabel("Reply")).toBeFocused();

  // Escape leaves the message box, and the letters run their commands again.
  await page.keyboard.press("h");
  await expect(page.getByLabel("Reply")).toHaveValue("h");
  await page.getByLabel("Reply").fill("");
  await page.keyboard.press("Escape");
  await expect(page.getByLabel("Reply")).not.toBeFocused();
  await page.keyboard.press("h");
  await expect(page.getByRole("button", { name: "history H" })).toHaveAttribute("aria-current", "page");
  await page.keyboard.press("c");
  await expect(page.getByRole("button", { name: "code C" })).toHaveAttribute("aria-current", "page");
  await page.keyboard.press("c");
  await expect(page.getByLabel("Reply")).toBeVisible();

  // A project is opened the same way, with a new thread to start. Enter right
  // behind the last letter chooses from what was typed, not from what was drawn.
  await page.keyboard.press("/");
  await expect(page.getByLabel("Search the menu")).toBeFocused();
  await page.keyboard.type("other");
  await page.keyboard.press("Enter");
  await expect(page.getByPlaceholder("What should Claude do in Other?")).toBeFocused();

  // Something no command holds says so, and Enter does nothing.
  await page.keyboard.press("/");
  await expect(page.getByLabel("Search the menu")).toBeFocused();
  await page.keyboard.type("zzzz");
  await expect(menu(page).getByText("Nothing matches.")).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(menu(page)).toBeVisible();
});

test("the keys and the menu start a new thread and show the running agents", async ({ page }) => {
  await addProject(page, "Sample");
  await page.getByLabel("New thread").fill("be slow");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.locator(".workspace-bar")).toContainText("1 running");

  // The menu marks the thread an agent is working in, and can stop it.
  await page.keyboard.press("Control+k");
  await expect(menu(page).getByRole("option", { name: /^be slow/ })).toContainText("running");
  await menu(page).getByRole("option", { name: /^Stop the agent/ }).click();
  await expect(page.getByText("Stopped by you.")).toBeVisible();
  await expect(page.locator(".workspace-bar")).not.toContainText("running");

  // N starts a new thread in the same project.
  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("n");
  await expect(page.getByPlaceholder("What should Claude do in Sample?")).toBeVisible();

  // The agent for new threads is changed from the menu too.
  await choose(page, "Use Grok for new threads");
  await expect(page.getByPlaceholder("What should Grok do in Sample?")).toBeVisible();
});

test("a thread whose agent stopped while it was out of sight waits to be looked at", async ({ page }) => {
  await addProject(page, "Sample");
  await page.getByLabel("New thread").fill("be slow");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.getByRole("status")).toHaveText(/Claude is working…/);

  // The agent is stopped while the files are in front of its thread.
  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("c");
  await expect(page.getByRole("button", { name: "code C" })).toHaveAttribute("aria-current", "page");
  await choose(page, "Stop the agent");

  await page.keyboard.press("Control+k");
  await expect(menu(page).locator(".menu-heading").first()).toHaveText("Waiting for you");
  await expect(menu(page).getByRole("option").first()).toHaveText(/be slow\s*Sample/);
  await page.keyboard.press("Escape");
  await expect(menu(page)).toHaveCount(0);

  // A new thread is not the one waiting, so the bar still names it.
  await page.keyboard.press("n");
  await expect(page.locator(".workspace-bar")).toContainText("waiting: be slow");

  // Opening it is looking at it.
  await page.keyboard.press("Control+k");
  await menu(page).getByRole("option", { name: /^be slow/ }).click();
  await expect(page.getByText("Stopped by you.")).toBeVisible();
  await expect(page.locator(".workspace-bar")).not.toContainText("waiting");
  await page.keyboard.press("Control+k");
  await expect(menu(page).locator(".menu-heading").first()).toHaveText("Threads");
});

test("the agent's own slash commands are listed and go to the message box", async ({ page }) => {
  await addProject(page, "Sample");

  // Before an agent has run, there are none to offer.
  await page.keyboard.press("Control+k");
  await expect(menu(page).locator(".menu-heading")).toHaveText(["Projects", "Actions"]);
  await page.keyboard.press("Escape");
  await expect(menu(page)).toHaveCount(0);

  await page.getByLabel("New thread").fill("hello");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.getByText(/^Done in/)).toBeVisible();

  // A slash in the empty message box and the start of a name, as in the agent's own program.
  await page.getByLabel("Reply").focus();
  await page.keyboard.press("/");
  await expect(page.getByLabel("Search the menu")).toBeFocused();
  await expect(menu(page).locator(".menu-heading").last()).toHaveText("Agent commands");
  await expect(menu(page).getByRole("option", { name: /^\/review/ })).toContainText("Claude command");
  await page.keyboard.type("comp");
  await expect(menu(page).getByRole("option").first()).toHaveText(/\/compact\s*Claude command/);
  await page.keyboard.press("Enter");

  // The command waits in the message box for what follows it, and is sent like any message.
  await expect(page.getByLabel("Reply")).toHaveValue("/compact ");
  await expect(page.getByLabel("Reply")).toBeFocused();
  await page.keyboard.type("keep the plan");
  await page.keyboard.press("Enter");
  await expect(page.getByLabel("Thread").getByText("/compact keep the plan", { exact: true })).toBeVisible();

  // A new thread borrows the commands until its own agent has run.
  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("n");
  await choose(page, "/review");
  await expect(page.getByLabel("New thread")).toHaveValue("/review ");
});

test("the theme is changed from the menu, which offers nothing about accounts", async ({ page }) => {
  const before = await page.locator("html").getAttribute("data-theme");
  await choose(page, before === "dark" ? "Light theme" : "Dark theme");
  await expect(page.locator("html")).not.toHaveAttribute("data-theme", before ?? "");

  await page.keyboard.press("Control+k");
  await expect(menu(page).getByRole("option", { name: /^Settings/ })).toHaveCount(0);
  await expect(menu(page).getByRole("option", { name: /^Sign out/ })).toHaveCount(0);
});

test("the phone layout brings the menu up from the bottom, to be tapped", async ({ page }) => {
  await addProject(page, "Sample");
  await page.getByLabel("New thread").fill("hello");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.getByText(/^Done in/)).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });

  await expect(page.locator(".workspace-bar")).toContainText("Sample · hello");
  await page.getByRole("button", { name: "Open menu" }).click();

  const sheet = menu(page);
  await expect(sheet).toBeVisible();
  const box = await sheet.boundingBox();
  expect(box?.x).toBe(0);
  expect(box?.width).toBe(390);
  expect((box?.y ?? 0) + (box?.height ?? 0)).toBe(844);
  await expect(sheet.locator("kbd").first()).toBeHidden();
  const row = await sheet.getByRole("option", { name: /^Sample/ }).boundingBox();
  expect(row?.height).toBeGreaterThanOrEqual(44);

  await sheet.getByRole("option", { name: /^Sample/ }).click();
  await expect(sheet).toHaveCount(0);
  await expect(page.getByPlaceholder("What should Claude do in Sample?")).toBeVisible();
});

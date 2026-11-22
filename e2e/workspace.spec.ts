// End-to-end specs for the workspace: a person adds a project and
// has an agent work in it.
//
// Each test has a Teryon service of its own, which starts a script that
// replays recordings in place of the agent programs. Nothing here reaches a model provider.

import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { Page } from "@playwright/test";

import { addProject, choose, commit, expect, git, menu, openWorkspace, put, test } from "./support";

test.beforeEach(openWorkspace);

/** The line above the message box about the open thread's change. */
function changeLine(page: Page) {
  return page.getByRole("region", { name: "This thread’s changes" });
}

test("Escape returns from code, history, usage and settings to the same thread and draft", async ({ page }) => {
  await addProject(page, "Sample", (folder) => {
    git(folder, "init", "-q", "-b", "main");
    put(folder, "README.md", "# Sample\n");
    commit(folder);
  });
  await page.getByLabel("New thread").fill("start a change fix-login");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.getByLabel("Thread").getByText("ready", { exact: true })).toBeVisible();
  await expect(changeLine(page)).toContainText("1 file changed");
  await page.getByLabel("Reply").fill("Keep this reply");

  for (const [key, selector] of [["c", "app-files"], ["h", "app-history"], ["u", "app-usage"], ["s", "app-settings"]]) {
    await page.getByLabel("Reply").press("Escape");
    await page.keyboard.press(key);
    await expect(page.locator(selector)).toBeVisible();

    await page.keyboard.press("Control+k");
    await expect(menu(page)).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(menu(page)).toHaveCount(0);
    await expect(page.locator(selector)).toBeVisible();

    for (const command of ["Discard changes", "Remove project"]) {
      await choose(page, command);
      await expect(page.locator("ion-alert")).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(page.locator("ion-alert")).toHaveCount(0);
      await expect(page.locator(selector)).toBeVisible();
    }

    if (key === "h") await page.locator("#history-filter").fill("Sample");
    await page.keyboard.press("Escape");
    await expect(page.locator(selector)).toHaveCount(0);
    await expect(page.getByLabel("Thread").getByText("start a change fix-login", { exact: true })).toBeVisible();
    await expect(page.getByLabel("Reply")).toHaveValue("Keep this reply");
  }
});

test("an agent works in a project and the thread fills in live", async ({ page }) => {
  await addProject(page, "Sample");

  await page.getByLabel("New thread").fill("hello");
  await page.getByRole("button", { name: "Run ↵" }).click();

  await expect(page.getByLabel("Thread").getByText("hello", { exact: true })).toBeVisible();
  await expect(page.getByLabel("Thread").getByText("ready", { exact: true })).toBeVisible();
  await expect(page.getByText(/^Done in \d+s · Fable 5\.2 · 25k tokens read, 4 written$/)).toBeVisible();
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
  const line = changeLine(page);
  // Ready to apply goes without a label: the button says so.
  await expect(line.getByRole("button")).toHaveText(["Ask Codex · default to review", "Apply to the project", "more"]);
  await expect(line.getByRole("status")).toHaveCount(0);
  await expect(line).toContainText("1 file changed");
  await expect(page.getByText("fix-login", { exact: true })).toHaveCount(0);
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

  // The menu lists the same, with its keys, and marks the thread as holding a change ready to apply.
  await page.keyboard.press("Control+k");
  await expect(menu(page).getByRole("option", { name: /^Copy location start a change fix-login W Y$/ })).toBeVisible();
  await expect(menu(page).getByRole("option", { name: /^Open folder start a change fix-login W F$/ })).toBeVisible();
  await expect(menu(page).locator(".menu-badge")).toHaveText("ready to apply");
  await expect(menu(page).getByRole("option", { name: /^Open in editor/ })).toHaveCount(0);
  await page.keyboard.press("Escape");

  // C shows the code of the thread's copy, where the agent's file is, and C again goes back.
  await expect(page.getByLabel("Reply")).toBeFocused();
  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("c");
  await expect(page.getByLabel("Repository files").getByRole("button", { name: "fix-login.txt" })).toBeVisible();
  await page.keyboard.press("c");
  await expect(page.getByLabel("Repository files")).toHaveCount(0);

  // "more" opens the menu on the change's commands alone.
  await line.getByRole("button", { name: "More about this thread’s changes" }).click();
  await expect(menu(page).locator(".menu-heading")).toHaveText(["This thread’s changes"]);
  await menu(page).getByRole("option", { name: /^See the changes/ }).click();
  await expect(page.getByLabel("Repository files").getByRole("button", { name: "fix-login.txt" })).toBeVisible();
  await expect(page.getByText(/\.teryon\/worktrees/)).toHaveCount(0);
});

test("another agent reviews a change in the same thread, and its findings go back to the author", async ({ page }) => {
  await addProject(page, "Sample", (folder) => {
    git(folder, "init", "-q", "-b", "main");
    put(folder, "README.md", "# Sample\n");
    commit(folder);
  });

  await page.getByLabel("New thread").fill("start a change fix-login");
  await page.getByRole("button", { name: "Run ↵" }).click();
  const line = changeLine(page);
  await line.getByRole("button", { name: "Ask Codex · default to review" }).click();

  // The review is part of the thread, set apart, and counted on the line.
  const thread = page.getByLabel("Thread");
  await expect(thread.locator(".review-divider")).toHaveText("Codex reviews the change.");
  await expect(thread.locator(".reviewer-entry .markdown").last()).toContainText("fix-login.txt has no test.");
  await expect(thread.locator(".reviewer-entry .speaker").first()).toHaveText("Codex · reviewing");
  await expect(line.getByRole("status")).toHaveText("2 findings");
  await expect(line).toContainText("from Codex");
  await expect(page.getByText("The change could not be reviewed.")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Who the message goes to" })).toHaveText("to Claude · default ▾");

  // The person can ask the reviewer, which leaves the count alone.
  await page.getByRole("button", { name: "Who the message goes to" }).click();
  await expect(page.getByLabel("Reply")).toHaveAttribute("placeholder", "Ask Codex");
  await page.getByLabel("Reply").fill("is the second one real?");
  await page.getByLabel("Reply").press("Enter");
  await expect(thread.locator(".user-entry.reviewer-entry")).toContainText("to Codex");
  await expect(thread.locator(".reviewer-entry .markdown").last()).toHaveText("done");
  await expect(line.getByRole("status")).toHaveText("2 findings");

  // Passing the findings drafts them for the author, without the count.
  await line.getByRole("button", { name: "Pass findings to Claude" }).click();
  await expect(page.getByRole("button", { name: "Who the message goes to" })).toHaveText("to Claude · default ▾");
  const reply = page.getByLabel("Reply");
  await expect(reply).toHaveValue(/^A reviewer looked at the change and found:\n\n1\. `fix-login\.txt` has no test\.\n2\. The redirect is not checked\.\n\nFix what/);
  await reply.press("Enter");
  await expect(line.getByRole("button", { name: "Ask Codex · default to review again" })).toBeVisible();

  // A second review finds nothing.
  await line.getByRole("button", { name: "Ask Codex · default to review again" }).click();
  await expect(line.getByRole("status")).toHaveText("Reviewed");
  await expect(line).toContainText("Codex found nothing");
  await expect(thread.locator(".review-divider")).toHaveCount(2);
});

test("an agent's change is applied to the project with the person's description", async ({ page }) => {
  const directory = await addProject(page, "Sample", (folder) => {
    git(folder, "init", "-q", "-b", "main");
    put(folder, "README.md", "# Sample\n");
    commit(folder);
  });

  await page.getByLabel("New thread").fill("start a change fix-login");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(changeLine(page).getByRole("button", { name: "Apply to the project" })).toBeVisible();

  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("w");
  await page.keyboard.press("a");
  const description = page.locator("ion-alert textarea");
  await expect(description).toHaveValue("start a change fix-login");
  await description.fill("Fix the login\n\nIt redirected twice.");
  await page.locator("ion-alert").getByRole("button", { name: "Apply" }).click();

  await expect(page.getByText("Applied to the project.", { exact: true }).first()).toBeVisible();
  await expect(page.getByLabel("Thread").getByText("Applied to the project.")).toBeVisible();
  await expect(changeLine(page).getByRole("status")).toHaveText("Applied");
  await expect(changeLine(page)).toContainText("in the project");
  await expect(changeLine(page).getByRole("button")).toHaveCount(0);
  await expect.poll(() => git(directory, "log", "-1", "--format=%B")).toBe("Fix the login\n\nIt redirected twice.");
  expect(existsSync(join(directory, "fix-login.txt"))).toBe(true);
  expect(existsSync(join(directory, ".teryon", "worktrees", "fix-login"))).toBe(false);
});

test("an agent's change is discarded once the person confirms", async ({ page }) => {
  const directory = await addProject(page, "Sample", (folder) => {
    git(folder, "init", "-q", "-b", "main");
    put(folder, "README.md", "# Sample\n");
    commit(folder);
  });

  await page.getByLabel("New thread").fill("start a change fix-login");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(changeLine(page)).toContainText("1 file changed");

  await choose(page, "Discard changes");
  await expect(page.locator("ion-alert")).toContainText("All changes in this thread will be lost: 1 changed file. This cannot be undone.");
  await page.locator("ion-alert").getByRole("button", { name: "Cancel" }).click();
  await expect(changeLine(page).getByRole("button", { name: "Apply to the project" })).toBeVisible();
  // The cancelled confirmation fades out; asking again before it has gone would show two.
  await expect(page.locator("ion-alert")).toHaveCount(0);

  await choose(page, "Discard changes");
  await page.locator("ion-alert").getByRole("button", { name: "Discard" }).click();
  await expect(page.getByLabel("Thread").getByText("Changes discarded.")).toBeVisible();
  await expect(changeLine(page).getByRole("status")).toHaveText("Discarded");
  await expect(changeLine(page).getByRole("button")).toHaveCount(0);
  expect(existsSync(join(directory, ".teryon", "worktrees", "fix-login"))).toBe(false);
  expect(git(directory, "rev-list", "--count", "main")).toBe("1");
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
  await page.getByRole("button", { name: "Change agent" }).click();
  await page.getByPlaceholder("What should Grok do in Sample?").fill("write markdown");
  await page.getByRole("button", { name: "Run ↵" }).click();

  const thread = page.getByLabel("Thread");
  await expect(thread.getByRole("heading", { name: "Findings" })).toBeVisible();
  await expect(thread.locator("pre code")).toContainText('println!("<hello>");');
});

test("a thread can be started with Codex", async ({ page }) => {
  await addProject(page, "Sample");

  await page.getByRole("button", { name: "Change agent" }).click();
  await page.getByPlaceholder("What should Codex do in Sample?").fill("use tools");
  await page.getByRole("button", { name: "Run ↵" }).click();

  await expect(page.getByText("shell  git worktree add ../sample-project--edit -b edit")).toBeVisible();
  await expect(page.getByText("edit  /work/sample-project--edit/a.txt")).toBeVisible();
  await expect(page.getByText(/^Done in \d+s · 105k tokens read, 587 written$/)).toBeVisible();
  await expect(page.locator(".thread-meta").getByText("Codex")).toBeVisible();
});

test("Codex models appear when searching from Codex default", async ({ page }) => {
  await addProject(page, "Sample");
  await page.getByRole("button", { name: "Change agent" }).click();
  await expect(page.getByRole("button", { name: "Change agent" })).toHaveText("Codex · default");

  await page.getByLabel("New thread").press("/");
  await page.getByRole("combobox", { name: "Search the menu" }).fill("codex");
  await menu(page).getByRole("option", { name: "Use Codex · GPT 6 astra for new threads", exact: true }).click();
  await expect(page.getByRole("button", { name: "Change agent" })).toHaveText("Codex · GPT 6 astra");
  await page.getByLabel("New thread").fill("hello");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.locator(".thread-meta")).toContainText("Codex · GPT 6 astra");
  await expect(page.getByText(/^Done in \d+s · GPT 6 astra ·/)).toBeVisible();

  await choose(page, "Use Codex · GPT 6 sol in this thread");
  await expect(page.locator(".thread-meta")).toContainText("Codex · GPT 6 sol");
});

test("a thread runs on the model chosen for it, and each turn says which model ran", async ({ page }) => {
  await addProject(page, "Sample");

  // Grok's models come from the program once the service has asked it.
  await page.keyboard.press("Control+k");
  await expect(menu(page).getByRole("option", { name: /^Use Grok · Grok 4\.6 for new threads/ })).toBeVisible();
  await page.keyboard.press("Escape");

  await choose(page, "Use Claude · Sonnet for new threads");
  await expect(page.getByRole("button", { name: "Change agent" })).toHaveText("Claude · Sonnet");
  await page.getByLabel("New thread").fill("hello");
  await page.getByRole("button", { name: "Run ↵" }).click();

  // The recording names the model that ran, which is what the summary shows.
  const summary = page.getByText(/^Done in \d+s · Fable 5\.2 · 25k tokens read, 4 written$/);
  await expect(summary).toBeVisible();
  await expect(summary).toHaveAttribute("title", "claude-fable-5-2");
  await expect(page.locator(".thread-meta")).toContainText("Claude · Sonnet");

  await choose(page, "Use Claude · Opus in this thread");
  await expect(page.locator(".thread-meta")).toContainText("Claude · Opus");
});

test("a thread moves to another agent, which carries on in it", async ({ page }) => {
  await addProject(page, "Sample");
  await page.getByLabel("New thread").fill("hello");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.getByText(/^Done in \d+s · Fable 5\.2/)).toBeVisible();

  await choose(page, "Use Codex · default in this thread");
  await expect(page.getByText("Codex takes over from Claude.")).toBeVisible();
  await expect(page.locator(".thread-meta")).toContainText("Codex · default");

  await page.getByLabel("Reply").fill("go on");
  await page.getByLabel("Reply").press("Enter");
  await expect(page.getByText("printf 'hi' > note.txt")).toBeVisible();
});

test("a thread can be started with Grok", async ({ page }) => {
  await addProject(page, "Sample");

  await page.getByRole("button", { name: "Change agent" }).click();
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

test("a new project's name starts as its directory's name", async ({ page }) => {
  const directory = realpathSync(mkdtempSync(join(tmpdir(), "teryon-e2e-named-")));
  const name = directory.split("/").pop() ?? directory;

  await choose(page, "New project");
  await page.getByLabel("Project directory").fill(directory);
  await expect(page.getByLabel("Project name")).toHaveValue(name);

  await page.getByRole("button", { name: "Add project" }).click();
  await expect(page.getByPlaceholder(`What should Claude do in ${name}?`)).toBeVisible();
});

test("a project directory can be picked from the list, and typed text narrows it", async ({ page }) => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "teryon-e2e-pick-")));
  mkdirSync(join(root, "payments"));
  mkdirSync(join(root, "website"));
  mkdirSync(join(root, ".hidden"));

  await choose(page, "New project");
  const directory = page.getByLabel("Project directory");
  const list = page.getByRole("listbox", { name: "Directories" });
  await directory.fill(root);

  await expect(list.getByRole("option", { name: "payments", exact: true })).toBeVisible();
  await expect(list.getByRole("option", { name: "website", exact: true })).toBeVisible();
  await expect(list.getByRole("option", { name: ".hidden", exact: true })).toHaveCount(0);

  await directory.fill(`${root}/.h`);
  await expect(list.getByRole("option", { name: ".hidden", exact: true })).toBeVisible();
  await expect(list.getByRole("option", { name: "payments", exact: true })).toHaveCount(0);

  await directory.fill(`${root}/pay`);
  await expect(list.getByRole("option", { name: "payments", exact: true })).toBeVisible();
  await expect(list.getByRole("option", { name: "website", exact: true })).toHaveCount(0);
  await directory.press("Enter");

  await expect(directory).toHaveValue(join(root, "payments"));
  await expect(page.getByLabel("Project name")).toHaveValue("payments");

  await page.getByLabel("Project name").fill("Custom");
  await list.getByRole("option", { name: "..", exact: true }).click();
  await expect(directory).toHaveValue(root);
  await expect(page.getByLabel("Project name")).toHaveValue("Custom");
});

test("enter on a directory name that matches nothing does not add the project", async ({ page }) => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "teryon-e2e-none-")));
  mkdirSync(join(root, "payments"));

  await choose(page, "New project");
  await page.getByLabel("Project name").fill("Nope");
  const directory = page.getByLabel("Project directory");
  await directory.fill(`${root}/zzz`);
  await expect(page.getByText("No directory matches.")).toBeVisible();
  await directory.press("Enter");

  await expect(page.getByText("The directory does not exist.")).toHaveCount(0);
  await expect(directory).toHaveValue(`${root}/zzz`);
  await expect(page.getByRole("heading", { name: "New project" })).toBeVisible();
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
  await choose(page, "Use Grok · default for new threads");
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

test("an agent's questions are answered with number keys, and the answers go to it as a message", async ({ page }) => {
  const directory = await addProject(page, "Sample");
  await page.getByLabel("New thread").fill("please ask questions");
  await page.getByRole("button", { name: "Run ↵" }).click();

  // The recommendations start chosen.
  const questions = page.getByRole("group", { name: "The agent’s questions" });
  await expect(questions).toBeFocused();
  await expect(questions.getByRole("button", { name: /In the thread/ })).toHaveAttribute("aria-pressed", "true");
  await expect(questions.getByRole("button", { name: /On the record/ })).toHaveAttribute("aria-pressed", "false");
  await expect(questions.getByRole("button", { name: /Thread list/ })).toHaveAttribute("aria-pressed", "true");
  await expect(questions.getByRole("button", { name: /Window title/ })).toHaveAttribute("aria-pressed", "false");
  await expect(questions.getByRole("button", { name: /Notification/ })).toHaveAttribute("aria-pressed", "true");

  // The thread waits for answers even while it is being looked at.
  await page.keyboard.press("Control+k");
  await expect(menu(page).locator(".menu-heading").first()).toHaveText("Waiting for you");
  await expect(menu(page).getByRole("option").first()).toContainText("waiting for your answers");
  await page.keyboard.press("Escape");

  // Escape puts the questions away for a message, and Q brings them back.
  await questions.focus();
  await page.keyboard.press("Escape");
  await expect(questions).toHaveCount(0);
  await expect(page.getByLabel("Reply")).toBeFocused();
  await page.getByLabel("Reply").press("Escape");
  await page.keyboard.press("q");
  await expect(questions).toBeFocused();

  // 2 picks the second answer and moves on; in a question taking several, numbers tick and untick.
  await page.keyboard.press("2");
  await page.keyboard.press("1");
  await page.keyboard.press("2");
  await page.keyboard.press("Enter");

  await expect(questions).toHaveCount(0);
  const thread = page.getByLabel("Thread");
  await expect(thread).toContainText("1. Where should answers be kept? → On the record");
  await expect(thread).toContainText("2. Which screens should show it? → Window title, Notification");
  await expect(page.getByLabel("Reply")).toHaveValue("");
  await expect
    .poll(() => readFileSync(join(directory, ".fake-agent-invocations"), "utf8"))
    .toContain("Answers to your questions: 1. Where should answers be kept? → On the record 2. Which screens should show it? → Window title, Notification");
  await expect(page.locator(".workspace-bar")).not.toContainText("waiting");
});

test("own words still in their field are sent by the button", async ({ page }) => {
  const directory = await addProject(page, "Sample");
  await page.getByLabel("New thread").fill("please ask questions");
  await page.getByRole("button", { name: "Run ↵" }).click();

  const questions = page.getByRole("group", { name: "The agent’s questions" });
  await expect(questions).toBeFocused();
  await page.keyboard.press("3");
  await page.getByLabel("Your own answer").fill("In a file of its own");
  await page.getByRole("button", { name: "send answers ↵" }).click();

  await expect(questions).toHaveCount(0);
  await expect(page.getByLabel("Thread")).toContainText("1. Where should answers be kept? → In a file of its own");
  await expect
    .poll(() => readFileSync(join(directory, ".fake-agent-invocations"), "utf8"))
    .toContain("1. Where should answers be kept? → In a file of its own 2.");
});

test("questions arriving while the menu is open leave the keyboard with the menu", async ({ page }) => {
  await addProject(page, "Sample");
  await page.getByLabel("New thread").fill("please ask questions slowly");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.getByRole("status")).toHaveText(/Claude is working…/);

  const before = await page.locator("html").getAttribute("data-theme");
  await page.keyboard.press("Control+k");
  const search = page.getByLabel("Search the menu");
  await search.fill("theme");
  const questions = page.getByRole("group", { name: "The agent’s questions" });
  await expect(questions).toHaveCount(1, { timeout: 10_000 });
  await expect(search).toBeFocused();

  // Enter goes to the menu, not to the questions.
  await page.keyboard.press("Enter");
  await expect(menu(page)).toHaveCount(0);
  await expect(page.locator("html")).not.toHaveAttribute("data-theme", before ?? "");
  await expect(questions).toHaveCount(1);
  await expect(page.getByLabel("Thread")).not.toContainText("Answers to your questions");
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
  await expect(menu(page).getByRole("option", { name: /^Settings/ })).toBeVisible();
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

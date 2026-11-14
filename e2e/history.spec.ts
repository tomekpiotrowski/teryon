// End-to-end specs for the history screen: a person opens a project's
// history, follows its commits and sees what one of them changed.
//
// Each project is a real git repository made for the test, because that is
// where the history is read from. Every commit is made at a moment the test
// chooses, so the times on screen are known.

import { expect, test, type Page } from "@playwright/test";
import { addProject, git, gitAt, openWorkspace, put } from "./support";

/** The start of the hour before this one, in seconds since 1970. Commits come minutes after it. */
const start = Math.floor(Date.now() / 3_600_000) * 3600 - 3600;

/** Commits everything in a directory, `minute` minutes after the start, and returns the commit's short name. */
function record(directory: string, minute: number, ...message: string[]): string {
  git(directory, "add", "-A");
  gitAt(directory, start + minute * 60, "commit", "-q", ...message.flatMap((paragraph) => ["-m", paragraph]));
  return git(directory, "rev-parse", "--short=7", "HEAD");
}

const adapterBefore = `import type { LegacyTxn } from "../legacy/types";

export class LedgerAdapter {
  async write(txn: LegacyTxn) {
    return db.legacy_txn.insert(txn);
  }
}
`;

const adapterAfter = `import type { LegacyTxn } from "../legacy/types";

export class LedgerAdapter {
  async write(txn: LegacyTxn) {
    const entry = toLedgerEntry(txn);
    return this.ledger.append(entry);
  }
}
`;

/** A repository with two commits on `main`. */
function repository(directory: string): void {
  git(directory, "init", "-q", "-b", "main");
  put(directory, "README.md", "# Payments\n");
  put(directory, "src/ledger/adapter.ts", adapterBefore);
  record(directory, 0, "Start the payments service");
  put(directory, "src/api/routes.ts", "export const routes = [];\n");
  record(directory, 1, "Add the routes");
}

/** The same repository on a branch `ledger-adapter` with two commits of its own. */
function branched(directory: string): void {
  repository(directory);
  git(directory, "switch", "-q", "-c", "ledger-adapter");
  put(directory, "src/ledger/entry.ts", "export function toLedgerEntry() {}\n");
  record(directory, 2, "Add the ledger entry");
  put(directory, "src/ledger/adapter.ts", adapterAfter);
  put(directory, "README.md", "# Payments\n\nWrites go to the ledger.\n");
  record(directory, 3, "Route writes through the ledger", "Writes used to go to the legacy table.\nThey are appended to the ledger now.");
}

function commits(page: Page) {
  return page.getByLabel("Commit history").locator(".history-commit");
}

test.beforeEach(openWorkspace);

test("a project's history lists its commits and shows what one changed", async ({ page }) => {
  let first = "";
  const directory = await addProject(page, "Payments", (folder) => {
    branched(folder);
    first = git(folder, "rev-parse", "--short=7", "HEAD~1");
  });

  await page.getByRole("button", { name: "history H" }).click();

  const list = page.getByLabel("Commit history");
  const changedFiles = page.getByRole("group", { name: "Changed files" });
  const change = page.getByRole("region", { name: /^Changes to / });

  // The branch's own commits come first, the ones the main branch has too after them.
  await expect(page.getByLabel("Branch")).toHaveValue("ledger-adapter");
  await expect(list.getByText("history · ledger-adapter")).toBeVisible();
  await expect(commits(page).locator(".history-subject")).toHaveText([
    "Route writes through the ledger",
    "Add the ledger entry",
    "Add the routes",
    "Start the payments service",
  ]);
  await expect(commits(page).locator(".history-tag")).toHaveText(["main", "main"]);
  await expect(commits(page).locator(".history-who")).toHaveText(["Tester", "Tester", "Tester", "Tester"]);
  await expect(commits(page).locator(".history-dot.own")).toHaveCount(2);
  await expect(commits(page).locator(".history-dot.base")).toHaveCount(2);
  await expect(commits(page).locator(".history-joint.fork")).toHaveCount(1);
  await expect(list.getByText("4 commits", { exact: true })).toBeVisible();
  await expect(page.getByText("2 ahead of main")).toBeVisible();

  // The screen opens on the newest commit, with its first changed file.
  await expect(commits(page).first()).toHaveAttribute("aria-current", "true");
  await expect(page.getByRole("heading", { name: "Route writes through the ledger" })).toBeVisible();
  await expect(page.getByText("Writes used to go to the legacy table.")).toBeVisible();
  await expect(page.getByText(`parent ${first}`)).toBeVisible();
  await expect(page.locator(".history-by")).toHaveText(/^Tester · /);
  await expect(changedFiles.getByRole("button")).toHaveText([/README\.md/, /src\/ledger/, /adapter\.ts/]);
  await expect(changedFiles.getByRole("button", { name: /README\.md/ })).toHaveAttribute("aria-current", "true");
  await expect(changedFiles.getByRole("button", { name: /README\.md/ })).toContainText("+2");
  await expect(change.locator(".files-line.added")).toHaveText(["2", "3Writes go to the ledger."]);

  // Another changed file shows its own lines, with the place of the change above them.
  await changedFiles.getByRole("button", { name: /adapter\.ts/ }).click();
  await expect(page.getByRole("heading", { name: "adapter.ts" })).toBeVisible();
  await expect(changedFiles.getByRole("button", { name: /adapter\.ts/ })).toContainText("+2 −1");
  await expect(changedFiles.getByRole("button", { name: /adapter\.ts/ }).getByLabel("modified")).toHaveText("M");
  await expect(change.locator(".history-hunk")).toHaveText("⋯@@ -2,6 +2,7 @@");
  await expect(change.locator(".files-line.removed")).toHaveText("−    return db.legacy_txn.insert(txn);");
  await expect(change.locator(".files-line.added")).toHaveText(["5    const entry = toLedgerEntry(txn);", "6    return this.ledger.append(entry);"]);
  await expect(change.locator(".files-line.added").first().locator(".code-keyword")).toHaveText("const");

  // Side by side, the old line faces the first of the new ones.
  await page.getByRole("button", { name: "Split" }).click();
  await expect(change.locator(".history-half.removed")).toHaveText("5    return db.legacy_txn.insert(txn);");
  await expect(change.locator(".history-half.added")).toHaveCount(2);
  await expect(change.locator(".history-half.blank")).toHaveCount(1);
  await page.getByRole("button", { name: "Unified" }).click();
  await expect(change.locator(".history-pair")).toHaveCount(0);

  // A folder of changed files can be closed.
  await changedFiles.getByRole("button", { name: "src/ledger" }).click();
  await expect(changedFiles.getByRole("button", { name: /adapter\.ts/ })).toHaveCount(0);
  await changedFiles.getByRole("button", { name: "src/ledger" }).click();

  // Another commit shows what it changed.
  await commits(page).filter({ hasText: "Add the ledger entry" }).click();
  await expect(page.getByRole("heading", { name: "Add the ledger entry" })).toBeVisible();
  await expect(change.locator(".files-line.added")).toHaveText(["1export function toLedgerEntry() {}"]);

  // The filter narrows the commits by a file they changed, and by who made them.
  await page.getByPlaceholder("Filter by message, author, thread or file…").fill("routes.ts");
  await expect(commits(page).locator(".history-subject")).toHaveText(["Add the routes"]);
  await expect(list.getByText("1 of 4 commits")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Add the routes" })).toBeVisible();
  await page.getByPlaceholder("Filter by message, author, thread or file…").fill("");
  await page.getByRole("button", { name: "Agents" }).click();
  await expect(commits(page)).toHaveCount(0);
  await expect(page.getByText("No commit matches.")).toBeVisible();
  await page.getByRole("button", { name: "People" }).click();
  await expect(commits(page)).toHaveCount(4);
  await page.getByRole("button", { name: "All", exact: true }).click();

  // A commit made meanwhile shows up without reloading.
  put(directory, "notes.md", "# Notes\n");
  record(directory, 4, "Add notes");
  await expect(commits(page).locator(".history-subject").first()).toHaveText("Add notes", { timeout: 15_000 });
  await expect(page.getByText("3 ahead of main")).toBeVisible();
});

test("the keys move between the thread, the history and the files, and through the commits", async ({ page }) => {
  await addProject(page, "Payments", repository);
  const composer = page.getByLabel("New thread");

  // Typed into the message box, an h is just a letter.
  await composer.fill("");
  await composer.press("h");
  await expect(composer).toHaveValue("h");
  await composer.fill("");

  await composer.blur();
  await page.keyboard.press("h");
  await expect(page.getByLabel("Commit history")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Add the routes" })).toBeVisible();

  // J and K step through the commits, and stop at the ends.
  await page.keyboard.press("j");
  await expect(page.getByRole("heading", { name: "Start the payments service" })).toBeVisible();
  await expect(commits(page).last()).toHaveAttribute("aria-current", "true");
  await page.keyboard.press("j");
  await expect(page.getByRole("heading", { name: "Start the payments service" })).toBeVisible();
  await page.keyboard.press("k");
  await expect(page.getByRole("heading", { name: "Add the routes" })).toBeVisible();

  // Typed into the filter, the same letters are text.
  await page.getByPlaceholder("Filter by message, author, thread or file…").fill("j");
  await expect(page.getByText("No commit matches.")).toBeVisible();
  await page.getByPlaceholder("Filter by message, author, thread or file…").fill("");
  await page.getByPlaceholder("Filter by message, author, thread or file…").blur();

  await page.keyboard.press("c");
  await expect(page.getByLabel("Repository files")).toBeVisible();
  await page.keyboard.press("h");
  await expect(page.getByLabel("Commit history")).toBeVisible();
  await page.keyboard.press("h");
  await expect(composer).toBeVisible();

  await page.getByRole("button", { name: "history H" }).click();
  await page.getByRole("button", { name: "code C" }).click();
  await expect(page.getByLabel("Repository files")).toBeVisible();
  await page.getByRole("button", { name: "history H" }).click();
  await expect(page.getByLabel("Commit history")).toBeVisible();
  await page.getByRole("button", { name: "history H" }).click();
  await expect(composer).toBeVisible();
});

test("two commits are compared with a shift-click", async ({ page }) => {
  let oldest = "";
  let newest = "";
  await addProject(page, "Payments", (folder) => {
    branched(folder);
    oldest = git(folder, "rev-parse", "--short=7", "HEAD~3");
    newest = git(folder, "rev-parse", "--short=7", "HEAD");
  });
  await page.getByRole("button", { name: "history H" }).click();
  await expect(page.getByRole("heading", { name: "Route writes through the ledger" })).toBeVisible();
  const changedFiles = page.getByRole("group", { name: "Changed files" });

  await commits(page).filter({ hasText: "Start the payments service" }).click({ modifiers: ["Shift"] });

  await expect(page.getByRole("heading", { name: `Changes from ${oldest} to ${newest}` })).toBeVisible();
  await expect(page.getByText("Start the payments service → Route writes through the ledger")).toBeVisible();
  await expect(changedFiles.getByRole("button")).toHaveText([/README\.md/, /src\/api/, /routes\.ts/, /src\/ledger/, /adapter\.ts/, /entry\.ts/]);
  await expect(commits(page).first()).toHaveAttribute("aria-current", "true");
  await expect(commits(page).last()).toHaveClass(/compared/);
  await expect(page.getByRole("button", { name: "Ask to revert" })).toHaveCount(0);

  // A plain click goes back to one commit.
  await commits(page).filter({ hasText: "Add the routes" }).click();
  await expect(page.getByRole("heading", { name: "Add the routes" })).toBeVisible();
  await expect(changedFiles.getByRole("button")).toHaveText([/src\/api/, /routes\.ts/]);
});

test("the files can be browsed as a commit left them", async ({ page }) => {
  let entry = "";
  await addProject(page, "Payments", (folder) => {
    branched(folder);
    entry = git(folder, "rev-parse", "--short=7", "HEAD~1");
  });
  await page.getByRole("button", { name: "history H" }).click();
  await commits(page).filter({ hasText: "Add the ledger entry" }).click();

  await page.getByRole("button", { name: `Browse files at ${entry}` }).click();

  const tree = page.getByLabel("Repository files");
  await expect(tree).toBeVisible();
  await expect(page.getByLabel("Branch").locator("option:checked")).toHaveText(entry);
  await expect(page.getByRole("heading", { name: "entry.ts" })).toBeVisible();
  await expect(page.getByText("added since main")).toBeVisible();
  // The later commit's change to the adapter is not there yet.
  await tree.getByRole("button", { name: "adapter.ts" }).click();
  await expect(page.getByRole("region", { name: /^Content of / }).getByText("return db.legacy_txn.insert(txn);")).toBeVisible();

  // Choosing a branch leaves the commit, in the files and in the history alike.
  await page.getByLabel("Branch").selectOption("main");
  await expect(page.getByLabel("Branch")).toHaveValue("main");
  await expect(page.getByRole("heading", { name: "adapter.ts" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Diff" })).toBeDisabled();
  await expect(tree.getByRole("button", { name: "entry.ts" })).toHaveCount(0);
  await page.getByRole("button", { name: "history H" }).click();
  await expect(page.getByLabel("Branch")).toHaveValue("main");
  await expect(commits(page).locator(".history-subject")).toHaveText(["Add the routes", "Start the payments service"]);
  await expect(commits(page).locator(".history-tag")).toHaveCount(0);
});

test("a commit made while an agent was working leads back to its thread", async ({ page }) => {
  const directory = await addProject(page, "Payments", repository);
  await page.getByLabel("New thread").fill("be slow");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.getByRole("status")).toHaveText(/Claude is working…/);
  put(directory, "notes.md", "# Notes\n");
  git(directory, "add", "-A");
  git(directory, "commit", "-q", "-m", "Add notes");
  const made = git(directory, "rev-parse", "--short=7", "HEAD");

  await page.getByRole("button", { name: "history H" }).click();

  const newest = commits(page).first();
  await expect(newest.locator(".history-subject")).toHaveText("Add notes");
  await expect(newest.locator(".history-who")).toHaveText("Claude");
  await expect(newest.locator(".history-tag")).toHaveText("be slow");
  await expect(newest.locator(".history-tag")).toHaveClass(/current/);
  await expect(page.locator(".history-by")).toHaveText(/^Claude agent · thread “be slow” · today /);
  await expect(page.getByText("Payments · 1 agent live")).toBeVisible();

  await page.getByRole("button", { name: "Agents" }).click();
  await expect(commits(page).locator(".history-subject")).toHaveText(["Add notes"]);
  await page.getByRole("button", { name: "People" }).click();
  await expect(commits(page).locator(".history-subject")).toHaveText(["Add the routes", "Start the payments service"]);
  await page.getByRole("button", { name: "All", exact: true }).click();
  await page.getByPlaceholder("Filter by message, author, thread or file…").fill("be slow");
  await expect(commits(page).locator(".history-subject")).toHaveText(["Add notes"]);

  // Reverting is asked of an agent: the request is put in the message box, not carried out.
  await page.getByRole("button", { name: "Ask to revert" }).click();
  await expect(page.getByLabel("Reply")).toHaveValue(`Revert commit \`${made}\` (Add notes). `);
  await expect(page.getByLabel("Reply")).toBeFocused();
  expect(git(directory, "rev-parse", "--short=7", "HEAD")).toBe(made);

  // From the commit, back to the thread it came from.
  await page.getByLabel("Reply").blur();
  await page.getByRole("button", { name: "history H" }).click();
  await page.getByRole("button", { name: "Open thread" }).click();
  await expect(page.getByRole("status")).toHaveText(/Claude is working…/);
  await page.getByRole("button", { name: "stop" }).click();
  await expect(page.getByText("Stopped by you")).toBeVisible();

  // The commit stays the agent's once its turn has ended.
  await page.getByRole("button", { name: "history H" }).click();
  await expect(commits(page).first().locator(".history-who")).toHaveText("Claude");
});

test("a project without commits, or without a repository, says so", async ({ page }) => {
  await addProject(page, "Empty", (folder) => {
    git(folder, "init", "-q", "-b", "main");
  });
  await page.getByRole("button", { name: "history H" }).click();
  await expect(page.getByText("There are no commits here yet.")).toBeVisible();
  await expect(page.getByLabel("Commit history").getByText("0 commits")).toBeVisible();
  await page.getByRole("button", { name: "history H" }).click();

  await addProject(page, "Plain", (folder) => put(folder, "README.md", "# Plain\n"));
  await page.getByRole("button", { name: "history H" }).click();
  await expect(page.getByText("This project's directory is not a git repository, so there are no files or history to show.")).toBeVisible();
  await expect(page.getByLabel("Branch")).toHaveCount(0);
  await page.getByRole("button", { name: "history H" }).click();
  await expect(page.getByLabel("New thread")).toBeVisible();
});

test("the phone layout stacks the commits above the open commit", async ({ page }) => {
  await addProject(page, "Payments", branched);
  await page.setViewportSize({ width: 390, height: 844 });

  await page.getByRole("button", { name: "history H" }).click();

  const list = await page.getByLabel("Commit history").boundingBox();
  const heading = await page.getByRole("heading", { name: "Route writes through the ledger" }).boundingBox();
  const files = await page.getByRole("group", { name: "Changed files" }).boundingBox();
  const change = await page.getByRole("region", { name: /^Changes to / }).boundingBox();
  expect(list && heading && list.y + list.height <= heading.y).toBeTruthy();
  expect(files && change && files.y + files.height <= change.y).toBeTruthy();
  await expect(page.getByRole("region", { name: /^Changes to / })).toBeInViewport();
});

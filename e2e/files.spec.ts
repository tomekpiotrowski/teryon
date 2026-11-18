// End-to-end specs for the file browser: a person opens a project's files,
// sees what changed and reads a file.
//
// Each project is a real git repository made for the test, because that is
// where the file browser reads from.

import { expect, test, addProject, commit, git, openWorkspace, put } from "./support";

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

/** A repository with one commit on `main`. */
function repository(directory: string): void {
  git(directory, "init", "-q", "-b", "main");
  put(directory, "README.md", "# Payments\n");
  put(directory, "src/api/routes.ts", "export const routes = [];\n");
  put(directory, "src/ledger/adapter.ts", adapterBefore);
  put(directory, "src/ledger/index.ts", 'export * from "./adapter";\n');
  commit(directory);
}

test.beforeEach(openWorkspace);

test("a project's files show what changed, and follow the disk", async ({ page }) => {
  const directory = await addProject(page, "Payments", (folder) => {
    repository(folder);
    put(folder, "src/ledger/adapter.ts", adapterAfter);
    put(folder, "src/ledger/entry.ts", "export function toLedgerEntry() {}\n");
  });

  await page.getByRole("button", { name: "code C" }).click();

  const tree = page.getByLabel("Repository files");
  const content = page.getByRole("region", { name: /^Content of / });
  const removedLine = content.locator(".files-line.removed");
  const addedLines = content.locator(".files-line.added");

  // The screen opens on the first changed file, with the folders around it open.
  await expect(page.getByLabel("Branch")).toHaveValue("main");
  await expect(tree.getByRole("button", { name: "src" })).toHaveAttribute("aria-expanded", "true");
  await expect(tree.getByRole("button", { name: "api" })).toHaveAttribute("aria-expanded", "false");
  await expect(tree.getByRole("button", { name: "adapter.ts" })).toHaveAttribute("aria-current", "true");
  await expect(tree.getByRole("button", { name: "adapter.ts" }).getByLabel("modified")).toHaveText("M");
  await expect(tree.getByRole("button", { name: "entry.ts" }).getByLabel("added")).toHaveText("A");
  await expect(tree.getByRole("button", { name: "routes.ts" })).toHaveCount(0);

  await expect(page.getByRole("heading", { name: "adapter.ts" })).toBeVisible();
  await expect(page.getByText("src / ledger /")).toBeVisible();
  await expect(page.getByText("modified, not committed")).toBeVisible();
  await expect(page.getByRole("button", { name: "Diff" })).toHaveAttribute("aria-pressed", "true");
  await expect(removedLine).toHaveText("−    return db.legacy_txn.insert(txn);");
  await expect(addedLines).toHaveText(["5    const entry = toLedgerEntry(txn);", "6    return this.ledger.append(entry);"]);
  await expect(page.getByText("TypeScript")).toBeVisible();
  await expect(page.getByText("8 lines")).toBeVisible();
  await expect(page.getByText("read-only")).toBeVisible();

  // The code is coloured: an added line as the file is, a removed line as it was.
  await expect(addedLines.first().locator(".code-keyword")).toHaveText("const");
  await expect(removedLine.locator(".code-keyword")).toHaveText("return");

  // As a file, the same content is shown without what was removed.
  await page.getByRole("button", { name: "File", exact: true }).click();
  await expect(removedLine).toHaveCount(0);
  await expect(addedLines).toHaveCount(0);
  await expect(content.getByText("const entry = toLedgerEntry(txn);")).toBeVisible();

  // The filter narrows the tree to matching files.
  await page.getByPlaceholder("Filter files…").fill("entry");
  await expect(tree.getByRole("button")).toHaveText([/src/, /ledger/, /entry\.ts/]);
  await tree.getByRole("button", { name: "entry.ts" }).click();
  await expect(page.getByText("added, not committed")).toBeVisible();
  await expect(addedLines).toHaveText(["1export function toLedgerEntry() {}"]);
  await page.getByPlaceholder("Filter files…").fill("");

  // An unchanged file has no changes to switch to.
  await tree.getByRole("button", { name: "api" }).click();
  await tree.getByRole("button", { name: "routes.ts" }).click();
  await expect(page.getByRole("heading", { name: "routes.ts" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Diff" })).toBeDisabled();
  await expect(content.getByText("export const routes = [];")).toBeVisible();

  // What an agent writes shows up without reloading.
  put(directory, "src/api/routes.ts", "export const routes = [];\nexport const more = [];\n");
  put(directory, "notes.md", "# Notes\n");
  await expect(tree.getByRole("button", { name: "notes.md" }).getByLabel("added")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByText("modified, not committed")).toBeVisible();
  await expect(content.getByText("export const more = [];")).toBeVisible({ timeout: 15_000 });
});

test("the C key and the code link switch between a thread and the files", async ({ page }) => {
  await addProject(page, "Payments", repository);
  const composer = page.getByLabel("New thread");

  // Typed into the message box, a c is just a letter.
  await composer.fill("");
  await composer.press("c");
  await expect(composer).toHaveValue("c");
  await composer.fill("");

  await composer.press("Escape");
  await page.keyboard.press("c");
  await expect(page.getByLabel("Repository files")).toBeVisible();
  await expect(page.getByText("Nothing has changed. Choose a file to read it.")).toBeVisible();

  await page.getByPlaceholder("Filter files…").fill("c");
  await expect(page.getByLabel("Repository files")).toBeVisible();
  await page.getByPlaceholder("Filter files…").blur();

  await page.keyboard.press("c");
  await expect(composer).toBeVisible();
  await page.getByRole("button", { name: "code C" }).click();
  await expect(page.getByLabel("Repository files")).toBeVisible();
  await page.getByRole("button", { name: "code C" }).click();
  await expect(composer).toBeVisible();
});

test("a file can be asked about in a thread", async ({ page }) => {
  await addProject(page, "Payments", repository);

  await page.getByRole("button", { name: "code C" }).click();
  await page.getByLabel("Repository files").getByRole("button", { name: "README.md" }).click();
  await expect(page.getByRole("heading", { name: "README.md" })).toBeVisible();
  await expect(page.getByText("Markdown")).toBeVisible();
  await page.getByRole("button", { name: "Ask about this file" }).click();

  await expect(page.getByLabel("New thread")).toHaveValue("About `README.md`: ");
  await expect(page.getByLabel("New thread")).toBeFocused();
});

test("another branch is compared with the main branch", async ({ page }) => {
  await addProject(page, "Payments", (folder) => {
    repository(folder);
    git(folder, "switch", "-q", "-c", "ledger-adapter");
    put(folder, "src/ledger/adapter.ts", adapterAfter);
    commit(folder);
    git(folder, "switch", "-q", "main");
  });

  await page.getByRole("button", { name: "code C" }).click();
  await expect(page.getByText("Nothing has changed. Choose a file to read it.")).toBeVisible();

  await page.getByLabel("Branch").selectOption("ledger-adapter");
  await expect(page.getByRole("heading", { name: "adapter.ts" })).toBeVisible();
  await expect(page.getByText("modified since main")).toBeVisible();
  await expect(page.locator(".files-line.removed")).toHaveText("−    return db.legacy_txn.insert(txn);");

  await page.getByLabel("Branch").selectOption("main");
  await expect(page.getByText("Nothing has changed. Choose a file to read it.")).toBeVisible();
});

test("a project that is not a git repository says so", async ({ page }) => {
  await addProject(page, "Plain", (folder) => put(folder, "README.md", "# Plain\n"));

  await page.getByRole("button", { name: "code C" }).click();

  await expect(page.getByText("This project's directory is not a git repository, so there are no files or history to show.")).toBeVisible();
  await expect(page.getByLabel("Branch")).toHaveCount(0);
  await page.getByRole("button", { name: "code C" }).click();
  await expect(page.getByLabel("New thread")).toBeVisible();
});

test("the phone layout stacks the tree above the file", async ({ page }) => {
  await addProject(page, "Payments", (folder) => {
    repository(folder);
    put(folder, "src/ledger/adapter.ts", adapterAfter);
  });
  await page.setViewportSize({ width: 390, height: 844 });

  await page.getByRole("button", { name: "code C" }).click();

  const tree = await page.getByLabel("Repository files").boundingBox();
  const heading = await page.getByRole("heading", { name: "adapter.ts" }).boundingBox();
  expect(tree && heading && tree.y + tree.height <= heading.y).toBeTruthy();
  await expect(page.getByRole("region", { name: /^Content of / })).toBeInViewport();
});

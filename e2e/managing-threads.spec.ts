// End-to-end specs for managing threads from the `teryon` command:
// messages sent from a terminal and from another thread show up live in the app.

import { execFileSync } from "node:child_process";
import { join } from "node:path";

import { addProject, expect, openWorkspace, startThread, test } from "./support";

const program = process.env.TERYON_PROGRAM ?? join(__dirname, "..", "target", "debug", "teryon");

/** Runs `teryon --home <home> ...` without inheriting any outer Teryon environment variables. */
function runTeryon(home: string, args: string[], actingThread?: string): string {
  const env = { ...process.env };
  delete env["TERYON_THREAD"];
  delete env["TERYON_TURN"];
  delete env["TERYON_DATA"];
  delete env["TERYON_HOME"];
  if (actingThread) env["TERYON_THREAD"] = actingThread;
  return execFileSync(program, ["--home", home, ...args], {
    env,
    encoding: "utf8",
    stdio: "pipe",
  }).trim();
}

test.beforeEach(openWorkspace);

test("a message sent with `teryon thread send` from a terminal shows up in the app", async ({ page, service }) => {
  await addProject(page, "Sample");
  await startThread(page, "hello");
  await expect(page.getByLabel("Thread").getByText("ready", { exact: true })).toBeVisible();

  const threads = JSON.parse(runTeryon(service.home, ["--json", "threads"])) as Array<{ id: string }>;
  expect(threads).toHaveLength(1);

  runTeryon(service.home, ["thread", "send", threads[0].id.slice(0, 8), "--wait", "sent from terminal"]);

  const thread = page.getByLabel("Thread");
  await expect(thread.getByText("sent from terminal", { exact: true })).toBeVisible();
  await expect(thread.locator(".message-source")).toHaveCount(0);
});

test("a message sent from another thread shows From «title» and opens that thread", async ({ page, service }) => {
  await addProject(page, "Sample");
  await startThread(page, "hello");
  await expect(page.getByLabel("Thread").getByText("ready", { exact: true })).toBeVisible();

  // Create a second thread that acts as the sender.
  const sender = JSON.parse(
    runTeryon(service.home, ["--json", "thread", "start", "--project", "Sample", "--title", "Coordinator", "--wait", "hello"]),
  ) as { thread: { id: string } };
  const threads = JSON.parse(runTeryon(service.home, ["--json", "threads"])) as Array<{ id: string; title: string }>;
  const target = threads.find((candidate) => candidate.id !== sender.thread.id)!;

  runTeryon(
    service.home,
    ["thread", "send", target.id.slice(0, 8), "--wait", "message from coordinator"],
    sender.thread.id,
  );

  const thread = page.getByLabel("Thread");
  await expect(thread.getByText("message from coordinator", { exact: true })).toBeVisible();
  const attribution = thread.getByRole("button", { name: "From «Coordinator»" });
  await expect(attribution).toBeVisible();

  await attribution.click();
  await expect(page.locator(".top-scope")).toHaveText("Coordinator");
});

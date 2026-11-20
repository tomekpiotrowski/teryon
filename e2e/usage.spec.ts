// End-to-end spec for the usage screen: after an agent's turn, its limits
// and the day's tokens show up.
//
// The fake Claude replays a recorded turn, which reports Claude's limits and
// 4 tokens written. The recording's windows reset on 14 November 2026, which
// has passed, so they show as started over.

import { addProject, expect, openWorkspace, test } from "./support";

test.beforeEach(openWorkspace);

test("the usage screen shows an agent's limits and the day's tokens after a turn", async ({ page }) => {
  await addProject(page, "Sample");
  await page.getByLabel("New thread").fill("hello");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.getByText(/^Done in \d+s · Fable 5\.2 · 25k tokens read, 4 written$/)).toBeVisible();

  await page.keyboard.press("Escape");
  await page.keyboard.press("u");

  const limits = page.getByRole("region", { name: "Limits" });
  await expect(limits.getByRole("meter", { name: "Claude, 5 hours" })).toHaveAttribute("aria-valuenow", "0");
  await expect(limits.getByRole("meter", { name: "Claude, week", exact: true })).toBeVisible();
  await expect(limits.getByText("reset since the last reading")).toHaveCount(3);
  await expect(limits.getByText(/^as of /)).toBeVisible();
  await expect(limits.getByText("Grok shows its limit only in its own window.")).toBeVisible();

  const days = page.getByRole("region", { name: "Teryon’s agents · last 14 days" });
  await expect(days.getByRole("button", { name: "Claude 1" })).toHaveAttribute("aria-pressed", "true");
  await expect(days.getByRole("listitem")).toHaveCount(14);
  await expect(days.getByRole("listitem").last()).toHaveAttribute("aria-label", /: 1 turn, 25k read, 4 written$/);
  await expect(days.getByRole("table").first()).toContainText("Sample");
  await expect(days.getByRole("table").last()).toContainText("Fable 5.2");

  await page.keyboard.press("u");
  await expect(page.getByRole("region", { name: "Limits" })).toHaveCount(0);
});

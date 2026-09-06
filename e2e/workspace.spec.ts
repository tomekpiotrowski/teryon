import { expect, test } from "@playwright/test";

const session = {
  id: "visual-test-user",
  email: "visual-test@example.com",
};

async function collapseDevtools(page: import("@playwright/test").Page): Promise<void> {
  const minimize = page.getByRole("dialog", { name: "Erno Devtools" }).getByRole("button", { name: "—" });
  if (await minimize.isVisible()) await minimize.click();
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript((user) => {
    window.localStorage.setItem("erno_refresh_token", "test-refresh-token");
    window.localStorage.setItem("erno_user", JSON.stringify(user));
    window.sessionStorage.setItem("erno_access_token", "test-access-token");
  }, session);
});

test("desktop workspace supports project and composer interactions", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await collapseDevtools(page);

  await expect(page.getByText("TERYON", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: /Payments migration/ })).toBeVisible();
  await page.getByRole("button", { name: /Docs sweep/ }).click();
  await expect(page.getByPlaceholder("What should Teryon do?")).toBeVisible();

  await page.getByPlaceholder("What should Teryon do?").fill("Audit retrying webhooks.");
  await page.getByRole("button", { name: "Run ↵" }).click();
  await expect(page.getByText("Audit retrying webhooks.")).toBeVisible();
  await expect(page.getByText("planner · drafting a plan · 3s")).toBeVisible();
});

test("phone workspace uses the mobile project drawer and theme control", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await collapseDevtools(page);

  await expect(page.getByRole("button", { name: "Open projects" })).toBeVisible();
  await page.getByRole("button", { name: "Open projects" }).click();
  await expect(page.locator(".project-navigation")).toBeInViewport();
  await page.getByRole("button", { name: /Onboarding rewrite/ }).click();
  await expect(page.locator(".mobile-header").getByText("Onboarding rewrite")).toBeVisible();

  await page.getByRole("button", { name: "Toggle color theme" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
});

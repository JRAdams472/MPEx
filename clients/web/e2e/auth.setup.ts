import { expect, test as setup } from "@playwright/test";

const AUTH_FILE = "e2e/.auth/admin.json";

/**
 * One real login through Dex as the admin user; every other spec reuses the
 * sealed session cookie via storageState instead of re-logging.
 */
setup("authenticate as admin", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveURL(/localhost:5556\/dex\/auth/);
  await page.fill('input[name="login"]', "admin@mpex.dev");
  await page.fill('input[name="password"]', "admin123");
  await page.click('button[type="submit"]');
  await expect(page.getByRole("heading", { name: "Dashboard" })).toBeVisible();
  await page.context().storageState({ path: AUTH_FILE });
});

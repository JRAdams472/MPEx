import { expect, test } from "@playwright/test";

/**
 * Baseline browser protections (MPE-97 phase 4): every document response
 * carries the hardening headers configured in next.config.ts.
 */
test("document responses carry security headers", async ({ page }) => {
  const res = await page.goto("/");
  expect(res).not.toBeNull();
  const headers = res!.headers();
  expect(headers["content-security-policy"]).toContain("default-src 'self'");
  expect(headers["x-content-type-options"]).toBe("nosniff");
  expect(headers["x-frame-options"]).toBe("DENY");
  expect(headers["referrer-policy"]).toBe("no-referrer");
});

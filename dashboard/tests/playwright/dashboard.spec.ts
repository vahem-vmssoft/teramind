import { test, expect } from '@playwright/test';

const PASSWORD = process.env.TMD_ADMIN_PASSWORD ?? 'hunter2hunter2';

test('login then visit all four views', async ({ page }) => {
  await page.goto('/dashboard/login');
  await page.fill('input[type="password"]', PASSWORD);
  await page.click('button:has-text("Sign in")');
  await expect(page).toHaveURL(/\/dashboard\/(activity|$)/);
  await expect(page.locator('h1')).toContainText('Activity');

  await page.click('text=Skills');
  await expect(page.locator('h1')).toContainText('Skills');

  await page.click('text=Members');
  await expect(page.locator('h1')).toContainText('Members');

  await page.click('text=Quality');
  await expect(page.locator('h1')).toContainText('Search quality');

  await page.click('text=Health');
  await expect(page.locator('h1')).toContainText('Health');
});

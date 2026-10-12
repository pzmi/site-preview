// Smoke spec for the harness (E2E-04): the real binary serves the fixture site to a real
// browser. Reload behaviour is E2E-05, E2E-06 and E2E-09.
import { expect, test } from './support/preview';

test('should_load_the_fixture_page_when_the_server_is_ready', async ({ page, preview }) => {
  // given
  expect(preview.ready).toMatchObject({ event: 'ready', watching: true });
  expect(preview.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/$/);

  // when
  const response = await page.goto(preview.url);

  // then
  expect(response?.status()).toBe(200);
  await expect(page.getByRole('heading', { name: 'Route planner' })).toBeVisible();
  await expect(page.locator('#routes')).not.toBeEmpty();
});

import { expect, test, type Page } from '@playwright/test';
import { categorizedGemOverview, gemAnalysis, gemOverview, prices, status } from '../src/testing/fixtures';

async function service(page: Page, options: {
  many?: boolean; failPrices?: boolean; lowConfidence?: boolean; categorized?: boolean; trimHistoryOnRefresh?: boolean;
} = {}) {
  let latestRequests = 0;
  const gems = options.categorized ? categorizedGemOverview() : gemOverview();
  if (options.lowConfidence) {
    gems.gems = gems.gems.map((gem) => ({ ...gem, sample_count: 2 }));
  }
  if (options.many) {
    const variants = gems.gems;
    gems.gems = Array.from({ length: 125 }, (_, index) =>
      variants.map((gem) => ({ ...gem, name: options.categorized
        ? `${String(index).padStart(3, '0')} ${gem.name}` : `Gem ${String(index).padStart(3, '0')}` }))).flat();
  }
  await page.route('**/api/v1/**', async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (path.endsWith('/leagues')) return route.fulfill({ json: [{ id: 'Standard', name: 'Standard' }, { id: 'Other', name: 'Other league' }] });
    if (path.endsWith('/status')) return route.fulfill({ json: status() });
    if (path.endsWith('/selection')) return route.fulfill({ json: { id: request.postDataJSON().id, name: 'Other league' } });
    if (path.endsWith('/prices')) return options.failPrices
      ? route.fulfill({ status: 502, body: '' }) : route.fulfill({ json: prices() });
    if (path.endsWith('/gems')) return route.fulfill({ json: gems });
    if (path.endsWith('/gem-analysis')) {
      const saved = new URL(request.url()).searchParams.has('snapshot');
      const analysis = gemAnalysis(gems, new URL(request.url()).searchParams.get('snapshot') === '1');
      if (!saved && ++latestRequests > 1 && options.trimHistoryOnRefresh) {
        analysis.snapshots = analysis.snapshots.filter((snapshot) => snapshot.id !== 1);
      }
      if (options.failPrices) {
        analysis.data.prices = null;
        analysis.data.price_error = 'Material prices returned HTTP 502';
      }
      return route.fulfill({ json: analysis });
    }
    if (path.endsWith('/markets')) return route.fulfill({ json: [{
      league: 'Standard', market_id: 'pair', market_pair: ['currency/chaos', 'currency/divine'],
      volume_traded: { chaos: 100 }, lowest_stock: {}, highest_stock: {},
      lowest_ratio: { chaos: 1 }, highest_ratio: { chaos: 2 }, next_change_id: 123,
    }] });
    throw new Error(`Unexpected API request: ${path}`);
  });
}

test('main tabs, league and refresh share one row without a branding header', async ({ page }) => {
  await service(page);
  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  await expect(page.getByText('Faustus', { exact: true })).toHaveCount(0);
  const header = page.locator('.app-header');
  await expect(header.getByRole('navigation', { name: 'Market pages' })).toBeVisible();
  for (const width of [1280, 720, 390]) {
    await page.setViewportSize({ width, height: 800 });
    const centers = await header.locator('nav button, select, #refresh').evaluateAll((elements) =>
      elements.map((element) => {
        const bounds = element.getBoundingClientRect();
        return bounds.y + bounds.height / 2;
      }));
    expect(centers).toHaveLength(5);
    expect(Math.max(...centers) - Math.min(...centers)).toBeLessThanOrEqual(1);
  }
});

test('web UI works without Tauri: prices, all gem strategies, search, Lens and history', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await service(page);
  await page.goto('/');
  await expect(page.getByRole('cell', { name: "Gemcutter's Prism", exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  await expect(page.locator('td.positive')).toHaveText('38.00c');
  await expect(page.locator('.gem-table th')).toHaveText(['Gem', 'Type', 'Buy', 'Sell', 'Net v', 'ROI', 'Listings']);
  await expect(page.locator('.gem-table tbody tr').first().locator('td')).toHaveCount(7);
  await expect(page.getByText('Buy level 1, sell the highest uncorrupted level.', { exact: false })).toHaveCount(0);
  await expect(page.locator('.gems .caption')).toHaveCount(0);
  const lensPrice = page.getByRole('textbox', { name: "Facetor's Lens unit price in Chaos" });
  await expect(lensPrice).toHaveValue('50');
  await expect(lensPrice).toHaveAttribute('placeholder', 'Chaos');
  await expect(page.getByRole('button', { name: /lens count/ })).toHaveCount(0);
  await page.getByRole('checkbox', { name: 'Lens', exact: true }).check();
  await expect(page.locator('td.negative')).toHaveText('-12.00c');
  await lensPrice.fill('5');
  await expect(page.locator('td.positive')).toHaveText('33.00c');
  await page.getByRole('button', { name: 'Add quality', exact: true }).click();
  await expect(page.locator('td.positive')).toHaveText('40.00c');
  await page.getByRole('button', { name: 'Vaal +1 level', exact: true }).click();
  await expect(page.locator('td.positive')).toHaveText('38.00c');
  await page.getByRole('button', { name: 'Vaal +3 quality', exact: true }).click();
  await expect(page.locator('td.positive')).toHaveText('18.00c');
  await page.getByRole('searchbox', { name: 'Search gems' }).fill('unmatched');
  await expect(page.getByRole('button', { name: 'Fireball', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Clear gem search' }).click();
  await page.getByRole('button', { name: 'Fireball', exact: true }).click();
  await page.getByRole('button', { name: 'Level flip', exact: true }).click();
  await expect(page.getByRole('searchbox', { name: 'Search gems' })).toHaveValue('Fireball');
  await page.getByRole('button', { name: 'GGG history', exact: true }).click();
  await expect(page.getByRole('cell', { name: 'chaos / divine', exact: true })).toBeVisible();
  const selection = page.waitForRequest((request) => request.url().endsWith('/selection') && request.method() === 'POST');
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('Other');
  expect((await selection).postDataJSON()).toEqual({ id: 'Other' });
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  expect(errors).toEqual([]);
});

test('limits rows to 50/100, sorts, resets pagination, and preserves native browser scrolling', async ({ page }) => {
  await service(page, { many: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  await expect(page.getByText('Showing 50 of 100', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Show more', exact: true }).click();
  await expect(page.getByText('Showing 100 of 100', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Gem', exact: true }).click();
  await expect(page.getByText('Showing 50 of 100', { exact: true })).toBeVisible();
  await expect(page.getByRole('columnheader', { name: /Gem/ })).toHaveAttribute('aria-sort', 'ascending');
  await page.getByRole('columnheader', { name: /Gem/ }).getByRole('button').click();
  await expect(page.getByRole('columnheader', { name: /Gem/ })).toHaveAttribute('aria-sort', 'descending');
  const table = page.locator('.gem-table');
  const box = await table.boundingBox();
  if (!box) throw new Error('Gem table is not visible');
  await page.mouse.move(box.x + 200, box.y + Math.min(box.height - 20, 200));
  await table.evaluate((element) => {
    element.ownerDocument.addEventListener('pointerdown', (event) => {
      element.setAttribute('data-middle-prevented', String(event.defaultPrevented));
      element.setAttribute('data-middle-captured', String(element.hasPointerCapture(event.pointerId)));
    }, { once: true });
  });
  await page.mouse.down({ button: 'middle' });
  await expect(table).toHaveAttribute('data-middle-prevented', 'false');
  await expect(table).toHaveAttribute('data-middle-captured', 'false');
  await page.mouse.up({ button: 'middle' });
  await page.keyboard.press('Escape');
  await page.mouse.wheel(0, 200);
  await expect.poll(() => table.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  await page.getByRole('searchbox', { name: 'Search gems' }).fill('Gem 001');
  await expect(page.getByText('Showing 1 of 1', { exact: true })).toBeVisible();
  expect(await table.evaluate((element) => element.scrollTop)).toBe(0);
});

test('renders partial provider errors and disconnected-service errors', async ({ page }) => {
  await service(page, { failPrices: true });
  await page.goto('/');
  await expect(page.getByRole('alert')).toContainText('HTTP 502');
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Fireball', exact: true })).toBeVisible();
  await page.unrouteAll();
  await page.route('**/api/v1/**', (route) => route.abort('connectionrefused'));
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('external market service');
});

test('confidence threshold updates filtering, badges and validation', async ({ page }) => {
  await service(page, { lowConfidence: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  await expect(page.getByRole('checkbox', { name: 'Include low confidence', exact: true })).not.toBeChecked();
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  await expect(page.getByRole('button', { name: 'Fireball', exact: true })).toHaveCount(0);
  await page.getByRole('checkbox', { name: 'Include low confidence', exact: true }).check();
  await expect(page.getByText('LOW CONF', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Fireball', exact: true })).toBeVisible();
  await page.getByRole('checkbox', { name: 'Include low confidence', exact: true }).uncheck();
  await expect(page.getByRole('button', { name: 'Fireball', exact: true })).toHaveCount(0);
  const threshold = page.getByRole('textbox', { name: 'Confidence threshold', exact: true });
  await expect(threshold).toHaveValue('10');
  await threshold.fill('2');
  await expect(page.getByRole('button', { name: 'Fireball', exact: true })).toBeVisible();
  await expect(page.getByText('LOW CONF', { exact: true })).toHaveCount(0);
  await threshold.fill('3');
  await expect(page.getByRole('button', { name: 'Fireball', exact: true })).toHaveCount(0);
  await page.getByRole('checkbox', { name: 'Include low confidence', exact: true }).check();
  await expect(page.getByText('LOW CONF', { exact: true })).toBeVisible();
  await threshold.fill('1.5');
  await expect(threshold).toHaveAttribute('aria-invalid', 'true');
  await expect(page.getByRole('alert')).toContainText('whole-number');
  await expect(page.locator('.gem-table tbody tr')).toHaveCount(0);
  await threshold.fill('2');
  await expect(threshold).toHaveAttribute('aria-invalid', 'false');
  await expect(page.getByRole('alert')).toHaveCount(0);
  await expect(page.locator('td.positive')).toHaveText('38.00c');
});

test('selects saved instants newest first, retains selection on refresh and keeps Current prices current', async ({ page }) => {
  await service(page, { trimHistoryOnRefresh: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  const instant = page.getByRole('combobox', { name: 'Gem price instant', exact: true });
  await expect(instant).toHaveValue('latest');
  const localDates = await page.evaluate(() => ['2026-11-09T12:00:00Z', '2026-11-08T12:00:00Z'].map((value) => {
    return new Date(value).toLocaleString('en-CA', {
      month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hourCycle: 'h23',
    });
  }));
  const latestLabel = await page.evaluate(() => `Latest ${new Date('2026-11-09T12:00:00Z').toLocaleTimeString('en-CA', {
    hour: '2-digit', minute: '2-digit', hourCycle: 'h23',
  })}`);
  await expect(instant.locator('option')).toHaveText([latestLabel, ...localDates]);
  await expect(instant.locator('option').nth(1)).toHaveAttribute('title',
    await page.evaluate(() => new Date('2026-11-09T12:00:00Z').toLocaleString('en-CA', {
      year: 'numeric', month: '2-digit', day: '2-digit',
      hour: '2-digit', minute: '2-digit', hourCycle: 'h23', timeZoneName: 'short',
    })));
  await expect(page.getByRole('button', { name: 'Earlier', exact: true })).toHaveCount(0);
  await expect(page.getByText('End: Now', { exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Add quality', exact: true }).click();
  await expect(page.locator('td.positive')).toHaveText('40.00c');
  await instant.selectOption('1');
  await expect(page.locator('td.positive')).toHaveText('20.00c');
  await expect(page.locator('.gem-table tbody tr').first().locator('td').nth(2)).toHaveText('20.00c');
  await expect(page.locator('.gem-table tbody tr').first().locator('td').nth(3)).toHaveText('50.00c');
  await expect(page.locator('.lens-controls .muted')).toContainText('GCP 0.50c');
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  await expect(instant).toHaveValue('1');
  await expect(page.locator('td.positive')).toHaveText('20.00c');
  await page.getByRole('button', { name: 'Current prices', exact: true }).click();
  await expect(page.getByRole('row').filter({ hasText: "Gemcutter's Prism" }).locator('td').nth(1)).toHaveText('1 chaos');
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  await instant.selectOption('latest');
  await expect(page.locator('td.positive')).toHaveText('40.00c');
  await instant.selectOption('2');
  await expect(instant).toBeEnabled();
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('Other');
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  await expect(instant).toHaveValue('latest');
});

for (const [timezoneId, label] of [
  ['America/New_York', 'Nov 9, 07:00'],
  ['Pacific/Auckland', 'Nov 10, 01:00'],
] as const) {
  test.describe(`snapshot menu in ${timezoneId}`, () => {
    test.use({ timezoneId, locale: 'en-US' });
    test('shows local time and date while retaining the snapshot ID', async ({ page }) => {
      await service(page);
      await page.goto('/');
      const localTimestamp = await page.evaluate(() =>
        new Date('2026-11-09T12:00:00Z').toLocaleString('en-CA', {
          year: 'numeric', month: '2-digit', day: '2-digit',
          hour: '2-digit', minute: '2-digit', hourCycle: 'h23', timeZoneName: 'short',
        }));
      await expect(page.locator('section[aria-label="Current prices"] .muted')).toContainText(`Updated ${localTimestamp}`);
      await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
      await expect(page.locator('.gems .caption')).toHaveCount(0);
      const instant = page.getByRole('combobox', { name: 'Gem price instant', exact: true });
      await expect(instant.locator('option').nth(1)).toHaveText(label);
      await expect(instant.locator('option').first()).toHaveText(`Latest ${label.split(', ')[1]}`);
      await expect(instant.locator('option').first()).toHaveAttribute('title', localTimestamp);
      await expect(instant.locator('option').nth(1)).toHaveAttribute('title', localTimestamp);
      await expect(instant).toHaveAttribute('title', 'Snapshot times are local');
      await instant.selectOption('2');
      await expect(instant).toHaveValue('2');
      await expect(instant.locator('option').first()).toHaveText(`Latest ${label.split(', ')[1]}`);
    });
  });
}

test('type multi-select matches any selected type and persists across search, strategies, instants and refresh', async ({ page }) => {
  await service(page, { categorized: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  await expect(page.locator('.gem-table tbody tr')).toHaveCount(5);
  const filter = page.locator('.type-filter > summary');
  const options = page.getByRole('group', { name: 'Gem types', exact: true });
  await expect(filter).toHaveText('Type: Allv');
  await filter.click();
  await expect(options.getByRole('checkbox')).toHaveCount(0);
  await expect(options.getByRole('listitem')).toHaveCount(7);
  await expect(options.getByRole('button', { name: 'Clear', exact: true })).toHaveCount(0);
  const all = options.getByRole('button', { name: 'All', exact: true });
  await expect(all).toHaveAttribute('aria-pressed', 'true');
  await expect(options.locator('.type-list button[aria-pressed="false"]')).toHaveCount(6);
  await options.getByRole('button', { name: 'Skill', exact: true }).click();
  await expect(all).toHaveAttribute('aria-pressed', 'false');
  await expect(options.locator('.type-list button[aria-pressed="true"]')).toHaveCount(1);
  await expect(page.locator('.gem-table tbody tr')).toHaveCount(1);
  await all.click();
  await expect(all).toHaveAttribute('aria-pressed', 'true');
  await expect(options.locator('.type-list button[aria-pressed="false"]')).toHaveCount(6);
  for (const [type, name] of [
    ['Skill', 'Fireball'], ['Support', 'Multistrike Support'], ['Exceptional', 'Empower Support'],
    ['Transfigured', 'Cleave of Rage'], ['Trarthus', 'Sunder of Trarthus'],
  ] as const) {
    await all.click();
    await options.getByRole('button', { name: type, exact: true }).click();
    await expect(options.getByRole('button', { name: type, exact: true })).toHaveAttribute('aria-pressed', 'true');
    await expect(page.locator('.gem-table tbody tr')).toHaveCount(1);
    await expect(page.getByRole('button', { name, exact: true })).toBeVisible();
    await expect(page.locator('.gem-table tbody tr td').nth(1)).toHaveText(type);
  }
  await options.getByRole('button', { name: 'Skill', exact: true }).click();
  await expect(filter).toHaveText('Type: 2 typesv');
  await expect(page.locator('.gem-table tbody tr')).toHaveCount(2);
  await page.getByRole('button', { name: 'Add quality', exact: true }).click();
  await expect(options).toBeHidden();
  await expect(filter).toHaveText('Type: 2 typesv');
  await expect(page.getByRole('button', { name: 'Sunder of Trarthus', exact: true })).toBeVisible();
  await page.getByRole('combobox', { name: 'Gem price instant', exact: true }).selectOption('1');
  await expect(page.locator('.gem-table tbody tr')).toHaveCount(2);
  await expect(filter).toHaveText('Type: 2 typesv');
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  await expect(filter).toHaveText('Type: 2 typesv');
  await page.getByRole('searchbox', { name: 'Search gems' }).fill('fireball');
  await expect(page.locator('.gem-table tbody tr')).toHaveCount(1);
  await expect(page.getByRole('button', { name: 'Fireball', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Clear gem search' }).click();
  await filter.click();
  await options.getByRole('button', { name: 'Skill', exact: true }).click();
  await options.getByRole('button', { name: 'Trarthus', exact: true }).click();
  await expect(filter).toHaveText('Type: Nonev');
  await expect(page.locator('.gem-table tbody tr')).toHaveCount(0);
  await expect(page.getByRole('status')).toContainText('Select at least one gem type');
  await all.click();
  await expect(filter).toHaveText('Type: Allv');
  await expect(page.locator('.gem-table tbody tr')).toHaveCount(5);
  await page.keyboard.press('Escape');
  await expect(options).toBeHidden();
  await page.getByRole('columnheader', { name: 'Type', exact: true }).getByRole('button').click();
  await expect(page.locator('.gem-table tbody tr').first().locator('td').nth(1)).toHaveText('Exceptional');
  await page.getByRole('columnheader', { name: 'Type ^', exact: true }).getByRole('button').click();
  await expect(page.locator('.gem-table tbody tr').first().locator('td').nth(1)).toHaveText('Trarthus');
});

test('type dropdown supports keyboard dismissal, focus and narrow viewports', async ({ page }) => {
  await service(page, { categorized: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  const filter = page.locator('.type-filter > summary');
  const options = page.getByRole('group', { name: 'Gem types', exact: true });
  await filter.focus();
  await page.keyboard.press('Enter');
  await expect(options).toBeVisible();
  await page.keyboard.press('Tab');
  await expect(options.getByRole('button', { name: 'All', exact: true })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(options).toBeHidden();
  await expect(filter).toBeFocused();
  await page.keyboard.press('Space');
  await expect(options).toBeVisible();
  await options.getByRole('button', { name: 'Unknown', exact: true }).focus();
  await page.keyboard.press('Tab');
  await expect(options).toBeHidden();
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 800 });
    await filter.click();
    const bounds = await options.boundingBox();
    if (!bounds) throw new Error('Expected type dropdown bounds');
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(width);
    await page.getByRole('searchbox', { name: 'Search gems' }).click();
    await expect(options).toBeHidden();
  }
});

test('compact gem controls stay narrow and type rows highlight selection without checkboxes', async ({ page }) => {
  await service(page, { categorized: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  const summary = page.locator('.type-filter > summary');
  for (const width of [1280, 720, 390]) {
    await page.setViewportSize({ width, height: 800 });
    for (const [locator, maximum] of [
      [summary, 132], [page.getByRole('combobox', { name: 'Gem price instant' }), 140],
      [page.getByRole('group', { name: 'Confidence filter' }), 190],
      [page.getByRole('textbox', { name: "Facetor's Lens unit price in Chaos" }), 70],
    ] as const) {
      const bounds = await locator.boundingBox();
      if (!bounds) throw new Error('Expected compact control bounds');
      expect(bounds.width).toBeLessThanOrEqual(maximum);
    }
  }
  await summary.click();
  const options = page.getByRole('group', { name: 'Gem types', exact: true });
  const skill = options.getByRole('button', { name: 'Skill', exact: true });
  await expect(skill).toHaveAttribute('aria-pressed', 'false');
  await skill.click();
  await expect(skill).toHaveAttribute('aria-pressed', 'true');
  await summary.hover();
  const selectedColor = await skill.evaluate((element) => getComputedStyle(element).backgroundColor);
  await skill.click();
  await expect(skill).toHaveAttribute('aria-pressed', 'false');
  await summary.hover();
  const unselectedColor = await skill.evaluate((element) => getComputedStyle(element).backgroundColor);
  expect(selectedColor).not.toBe(unselectedColor);
  await expect(options.getByRole('checkbox')).toHaveCount(0);
});

test('repeated type toggles with a large market remain responsive without page errors or crashes', async ({ page }) => {
  const errors: string[] = [];
  let crashed = false;
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('crash', () => { crashed = true; });
  await service(page, { many: true, categorized: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  await expect(page.locator('.gem-table tbody tr')).toHaveCount(50);
  await page.locator('.type-filter > summary').click();
  const options = page.getByRole('group', { name: 'Gem types', exact: true });
  for (let cycle = 0; cycle < 3; cycle++) {
    for (const type of ['Skill', 'Support', 'Exceptional', 'Transfigured', 'Trarthus', 'Unknown']) {
      const all = options.getByRole('button', { name: 'All', exact: true });
      await all.click();
      await expect(all).toHaveAttribute('aria-pressed', 'true');
      const button = options.getByRole('button', { name: type, exact: true });
      await button.click();
      await expect(button).toHaveAttribute('aria-pressed', 'true');
      await expect(all).toHaveAttribute('aria-pressed', 'false');
      await button.click();
      await expect(button).toHaveAttribute('aria-pressed', 'false');
    }
  }
  await options.getByRole('button', { name: 'All', exact: true }).click();
  await expect(page.locator('.gem-table tbody tr')).toHaveCount(50);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  expect(errors).toEqual([]);
  expect(crashed).toBe(false);
});

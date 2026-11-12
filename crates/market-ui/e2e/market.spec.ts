import { expect, test, type Page } from '@playwright/test';
import { categorizedGemOverview, gemAnalysis, gemOverview, prices, status, economyCategories, economyOverview, economyArchive, economyLine } from '../src/testing/fixtures';

async function service(page: Page, options: {
  many?: boolean; failPrices?: boolean; lowConfidence?: boolean; categorized?: boolean; trimHistoryOnRefresh?: boolean;
} = {}) {
  let latestRequests = 0;
  const archives = [economyArchive('Ancestors', 'completed'), economyArchive('Mirage')];
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
    if (path.includes('/economy/')) {
      const query = new URL(request.url()).searchParams;
      if (path.endsWith('/categories')) return route.fulfill({ json: economyCategories() });
      if (path.endsWith('/leagues')) return route.fulfill({ json: [{ id: 'Standard', name: 'Standard', live: true }, { id: 'Other', name: 'Other league', live: true }, { id: 'Ancestors', name: 'Ancestors', live: true, archived: true }] });
      if (path.endsWith('/dates')) return route.fulfill({ json: query.get('league') === 'Standard'
        ? ['2026-11-09'] : ['2026-07-16', '2026-06-25'] });
      if (path.endsWith('/overview') || path.endsWith('/refresh')) return options.failPrices
        ? route.fulfill({ status: 502, body: 'Provider unavailable' }) : route.fulfill({ json: economyOverview(
        query.get('category') ?? 'Currency', query.get('source') === 'stash' ? 'stash' : 'exchange',
        query.get('league') ?? 'Standard', query.get('date') ?? '') });
      if (path.endsWith('/history')) return route.fulfill({ json: { points: [
        { date: '2026-07-15', price: '1', currency: 'Chaos Orb', granularity: 'daily' },
        { date: '2026-07-16', price: '2', currency: 'Chaos Orb', granularity: 'daily' },
      ], coverage: { start: '2026-07-15', end: '2026-07-16' } } });
      if (path.endsWith('/archives')) return route.fulfill({ json: archives });
      if (path.endsWith('/import')) {
        const archive = economyArchive(request.postDataJSON().name, 'completed');
        archives.splice(1, 1, archive);
        return route.fulfill({ json: archive });
      }
    }
    if (path.endsWith('/leagues')) return route.fulfill({ json: [{ id: 'Standard', name: 'Standard' }, { id: 'Other', name: 'Other league' }] });
    if (path.endsWith('/status')) return route.fulfill({ json: status() });
    if (path.endsWith('/selection')) return route.fulfill({ json: { id: request.postDataJSON().id, name: 'Other league' } });
    if (path.endsWith('/prices')) return options.failPrices
      ? route.fulfill({ status: 502, body: '' }) : route.fulfill({ json: prices() });
    if (path.endsWith('/gems')) return route.fulfill({ json: gems });
    if (path.endsWith('/gem-analysis') || path.endsWith('/gem-analysis/refresh')) {
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

test('economy categories, facets, source views, details and responsive layout', async ({ page }) => {
  await service(page);
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/');
  await page.getByRole('button', { name: 'Stash', exact: true }).click();
  await page.getByRole('button', { name: 'Skill Gems', exact: true }).click();
  await expect(page.locator('.economy-table tbody tr')).toHaveCount(2);
  await page.getByRole('combobox', { name: 'Gem level', exact: true }).selectOption('21');
  expect(errors).toEqual([]);
  await expect(page.locator('.economy-table tbody tr')).toHaveCount(1);
  await expect(page.locator('.economy-item')).toContainText('Corrupted');
  await page.locator('.economy-item').click();
  await expect(page.getByRole('article', { name: 'Item details' })).toBeVisible();
  const breadcrumb = page.getByRole('navigation', { name: 'Item breadcrumb', exact: true });
  await expect(breadcrumb.getByRole('heading', { level: 1 })).toHaveText('Skill Gems > Fireball');
  await expect(breadcrumb.locator('[aria-current="page"]')).toHaveText('Fireball');
  await expect(page.getByRole('button', { name: /^Back to / })).toHaveCount(0);
  await expect(page.getByRole('img', { name: 'Saved price history in Chaos Orb' })).toBeVisible();
  await expect(page.locator('.history-price-line')).toHaveAttribute('d', /^M.+L/);
  await page.getByText('History data (2 observations)', { exact: true }).click();
  await expect(page.getByRole('table', { name: 'Historical price observations' }).locator('tbody tr')).toHaveCount(2);
  await breadcrumb.getByRole('button', { name: 'Skill Gems', exact: true }).click();
  await expect(breadcrumb).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Skill Gems', exact: true, level: 1 })).toBeVisible();
  await expect(page.getByRole('combobox', { name: 'Gem level', exact: true })).toHaveValue('21');
  await page.getByRole('button', { name: 'Maps', exact: true }).click();
  await page.getByRole('combobox', { name: 'Map tier', exact: true }).selectOption('16');
  await expect(page.locator('.economy-table')).toContainText('Dunes Map');
  await page.setViewportSize({ width: 390, height: 800 });
  await expect(page.getByRole('combobox', { name: 'Economy category', exact: true })).toBeVisible();
  await page.getByRole('combobox', { name: 'Economy category', exact: true }).selectOption('Currency');
  await expect(page.locator('.economy-table')).toContainText('Vaal Orb');
  expect(errors).toEqual([]);
});

test('economy groups separate items and unique equipment while keeping unique maps in Atlas', async ({ page }) => {
  await service(page);
  await page.goto('/');
  const sidebar = page.getByRole('navigation', { name: 'Economy categories', exact: true });
  const group = (name: string) => sidebar.locator('section').filter({
    has: page.getByRole('heading', { name, exact: true, level: 2 }),
  });
  await expect(sidebar.getByRole('heading', { level: 2 })).toHaveText(['General', 'Items', 'Uniques', 'Atlas']);
  await expect(group('General').getByRole('button')).toHaveText(['Currency', 'Fragments']);
  await expect(group('Items').getByRole('button')).toHaveText(['Skill Gems', 'Wombgifts', 'Corpses', 'Incubators']);
  await expect(group('Uniques').getByRole('button')).toHaveText(['Unique Weapons']);
  await expect(group('Atlas').getByRole('button')).toHaveText(['Maps', 'Unique Maps']);
  for (const name of ['Wombgifts', 'Corpses', 'Incubators', 'Unique Weapons', 'Unique Maps']) {
    await sidebar.getByRole('button', { name, exact: true }).click();
    await expect(page.getByRole('heading', { name, exact: true, level: 1 })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Stash', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Exchange', exact: true })).toHaveCount(0);
  }
  await page.setViewportSize({ width: 390, height: 800 });
  const category = page.getByRole('combobox', { name: 'Economy category', exact: true });
  await expect(category).toBeVisible();
  await expect(category.locator('optgroup')).toHaveCount(4);
  await expect(category.locator('optgroup[label="General"] option')).toHaveText(['Currency', 'Fragments']);
  await expect(category.locator('optgroup[label="Items"] option')).toHaveText(['Skill Gems', 'Wombgifts', 'Corpses', 'Incubators']);
  await expect(category.locator('optgroup[label="Uniques"] option')).toHaveText(['Unique Weapons']);
  await expect(category.locator('optgroup[label="Atlas"] option')).toHaveText(['Maps', 'Unique Maps']);
  await category.selectOption('Wombgift');
  await expect(page.getByRole('heading', { name: 'Wombgifts', exact: true, level: 1 })).toBeVisible();
});

test('price captions are omitted and Clear filters only appears with category-specific filters', async ({ page }) => {
  await service(page);
  await page.goto('/');
  await expect(page.locator('.economy-table tbody tr')).toHaveCount(2);
  const clear = page.getByRole('button', { name: 'Clear filters', exact: true });
  await expect(clear).toHaveCount(0);
  await page.getByRole('searchbox', { name: 'Search economy items', exact: true }).fill('Vaal');
  await page.getByRole('combobox', { name: 'Price currency', exact: true }).selectOption('chaos');
  await expect(clear).toHaveCount(0);
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('archive:Ancestors');
  await expect(page.getByRole('combobox', { name: 'As of', exact: true })).toHaveValue('2026-07-16');
  await expect(page.locator('.economy-count, .history-provenance')).toHaveCount(0);
  await expect(page.getByText(/^Icons: poe.ninja item metadata/)).toHaveCount(0);
  await expect(page.getByText(/Saved price history; volume and listing counts/)).toHaveCount(0);
  await expect(clear).toHaveCount(0);
  await page.locator('.economy-item').first().click();
  await expect(page.getByRole('slider', { name: 'Period start', exact: true })).toBeVisible();
  await expect(page.getByText(/^Native quote units\./)).toHaveCount(0);
  await expect(page.getByText(/^Drag anywhere on the overview graph/)).toHaveCount(0);
  await page.getByRole('navigation', { name: 'Item breadcrumb', exact: true })
    .getByRole('button', { name: 'Currency', exact: true }).click();
  await page.getByRole('button', { name: 'Skill Gems', exact: true }).click();
  await page.getByRole('combobox', { name: 'Gem level', exact: true }).selectOption('21');
  await expect(page.locator('.economy-table tbody tr')).toHaveCount(1);
  await clear.click();
  await expect(page.getByRole('combobox', { name: 'Gem level', exact: true })).toHaveValue('');
  await expect(page.locator('.economy-table tbody tr')).toHaveCount(2);
  await page.getByRole('button', { name: 'Maps', exact: true }).click();
  await expect(page.getByRole('combobox', { name: 'Map tier', exact: true })).toBeVisible();
  await expect(clear).toBeVisible();
  await page.getByRole('button', { name: 'Unique Weapons', exact: true }).click();
  await expect(page.getByRole('combobox', { name: 'Item type', exact: true })).toBeVisible();
  await expect(clear).toBeVisible();
  await page.route('**/api/v1/economy/overview?**', (route) => {
    const query = new URL(route.request().url()).searchParams;
    const overview = economyOverview(query.get('category')!, 'stash', query.get('league')!);
    overview.lines = overview.lines.map((line) => ({ ...line, listings: 12 }));
    return route.fulfill({ json: overview });
  });
  await page.getByRole('button', { name: 'Currency', exact: true }).click();
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('Standard');
  await page.getByRole('textbox', { name: 'Minimum listings', exact: true }).fill('99');
  await expect(page.locator('.economy-table tbody tr')).toHaveCount(0);
  await clear.click();
  await expect(page.getByRole('textbox', { name: 'Minimum listings', exact: true })).toHaveValue('');
  await expect(page.locator('.economy-table tbody tr')).toHaveCount(2);
});

test('the whole price page scrolls instead of the item table on desktop and mobile', async ({ page }) => {
  await service(page);
  await page.route('**/api/v1/economy/overview?**', (route) => {
    const overview = economyOverview();
    overview.lines = Array.from({ length: 150 }, (_, index) =>
      economyLine({ id: `item-${index}`, name: `Currency ${String(index).padStart(3, '0')}` }));
    return route.fulfill({ json: overview });
  });
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 640 });
    await page.goto('/#/prices');
    await page.reload();
    const prices = page.getByRole('region', { name: 'Current prices', exact: true });
    const table = page.locator('.economy-table');
    const heading = page.locator('.economy-heading');
    const sidebar = page.getByRole('navigation', { name: 'Economy categories', exact: true });
    await expect(table.locator('tbody tr')).toHaveCount(100);
    expect(await table.evaluate((element) => element.scrollHeight <= element.clientHeight + 1)).toBe(true);
    expect(await prices.evaluate((element) => element.scrollHeight > element.clientHeight)).toBe(true);
    const headingTop = (await heading.boundingBox())!.y;
    const sidebarTop = width > 760 ? (await sidebar.boundingBox())!.y : null;
    await table.locator('tbody tr').first().hover();
    await page.mouse.wheel(0, 480);
    await expect.poll(() => prices.evaluate((element) => element.scrollTop)).toBeGreaterThan(100);
    expect((await heading.boundingBox())!.y).toBeLessThan(headingTop - 100);
    if (sidebarTop !== null) {
      expect((await sidebar.boundingBox())!.y).toBeLessThan(sidebarTop - 100);
      expect(await sidebar.evaluate((element) => element.scrollTop)).toBe(0);
    }
    expect(await table.evaluate((element) => element.scrollTop)).toBe(0);
    await page.getByRole('button', { name: 'Show 100 more', exact: true }).click();
    await expect(table.locator('tbody tr')).toHaveCount(150);
    await table.locator('tbody tr').last().scrollIntoViewIfNeeded();
    await expect(table.locator('tbody tr').last()).toBeVisible();
    await expect(page.getByRole('button', { name: 'Show 100 more', exact: true })).toHaveCount(0);
  }
});

test('graph handles drag both period edges and history scrolls with the detail page', async ({ page }) => {
  await service(page);
  const points = Array.from({ length: 40 }, (_, index) => ({
    date: new Date(Date.UTC(2026, 6, 1 + index)).toISOString().slice(0, 10),
    price: String(index + 1), currency: 'Chaos Orb', granularity: 'daily',
  }));
  await page.route('**/api/v1/economy/history?**', (route) => route.fulfill({
    json: { points, coverage: { start: points[0]!.date, end: points.at(-1)!.date } },
  }));
  await page.goto('/');
  await page.locator('.economy-item').first().click();
  await expect(page.getByText('History data (40 observations)', { exact: true })).toBeVisible();
  const start = page.getByRole('slider', { name: 'Period start', exact: true });
  const end = page.getByRole('slider', { name: 'Period end', exact: true });
  const brush = await page.locator('.history-brush').boundingBox();
  expect(brush).not.toBeNull();
  expect(brush!.height).toBeLessThanOrEqual(40);
  expect((await start.boundingBox())!.width).toBeLessThanOrEqual(14);
  await expect(page.getByRole('button', { name: 'Reset period', exact: true })).toHaveCount(0);
  await start.scrollIntoViewIfNeeded();
  const box = await page.locator('.history-brush').boundingBox();
  const startBox = await start.boundingBox();
  await page.mouse.move(startBox!.x + startBox!.width / 2, startBox!.y + startBox!.height / 2);
  await page.mouse.down();
  await page.mouse.move(box!.x + box!.width * .25, box!.y + box!.height / 2, { steps: 8 });
  await page.mouse.up();
  await expect(start).toHaveAttribute('aria-valuenow', '10');
  const endBox = await end.boundingBox();
  await page.mouse.move(endBox!.x + endBox!.width / 2, endBox!.y + endBox!.height / 2);
  await page.mouse.down();
  await page.mouse.move(box!.x + box!.width * .75, box!.y + box!.height / 2, { steps: 8 });
  await page.mouse.up();
  await expect(end).toHaveAttribute('aria-valuenow', '29');
  await page.getByText('History data (20 observations)', { exact: true }).click();
  const table = page.getByRole('table', { name: 'Historical price observations' });
  await expect(table.locator('tbody tr')).toHaveCount(20);
  expect(await page.locator('.history-data').evaluate((element) =>
    element.clientHeight >= element.scrollHeight - 1)).toBe(true);
  await end.focus();
  await page.keyboard.press('ArrowLeft');
  await expect(end).toHaveAttribute('aria-valuenow', '28');
  await end.dblclick();
  await expect(table.locator('tbody tr')).toHaveCount(40);
  await start.press('ArrowRight');
  await expect(start).toHaveAttribute('aria-valuenow', '1');
  await start.dblclick();
  await expect(start).toHaveAttribute('aria-valuenow', '0');
  await end.press('ArrowLeft');
  await end.press('Enter');
  await expect(end).toHaveAttribute('aria-valuenow', '39');
  await table.locator('tbody tr').last().scrollIntoViewIfNeeded();
  await expect(table.locator('tbody tr').last()).toBeVisible();
  expect(await page.locator('.item-detail').evaluate((element) =>
    element.scrollHeight <= element.clientHeight + 1)).toBe(true);
  expect(await page.getByRole('region', { name: 'Current prices', exact: true })
    .evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
});

test('selected history dates persist across items, live leagues and archive datasets', async ({ page }) => {
  await service(page);
  await page.route('**/api/v1/economy/history?**', (route) => {
    const query = new URL(route.request().url()).searchParams;
    const days = query.get('league') === 'Other' ? [5, 6] : query.get('league') === 'Ancestors' ? [4, 5, 6, 7, 8, 9]
      : query.get('item') === 'vaal' ? [2, 3, 5, 8, 10] : Array.from({ length: 11 }, (_, index) => index + 1);
    const points = days.map((day) => ({
      date: `2026-07-${String(day).padStart(2, '0')}`, price: String(day), currency: 'Chaos Orb', granularity: 'daily',
    }));
    return route.fulfill({ json: { points, coverage: { start: points[0]!.date, end: points.at(-1)!.date } } });
  });
  await page.goto('/');
  await page.locator('.economy-item').filter({ hasText: "Gemcutter's Prism" }).click();
  const start = page.getByRole('slider', { name: 'Period start', exact: true });
  const end = page.getByRole('slider', { name: 'Period end', exact: true });
  await start.press('ArrowRight');
  await start.press('ArrowRight');
  await end.press('ArrowLeft');
  await end.press('ArrowLeft');
  await end.press('ArrowLeft');
  const period = page.locator('.history-period > p').first();
  await expect(period).toHaveText('Selected period: 2026-07-03 – 2026-07-08');
  await page.getByRole('navigation', { name: 'Item breadcrumb', exact: true }).getByRole('button', { name: 'Currency', exact: true }).click();
  await page.locator('.economy-item').filter({ hasText: 'Vaal Orb' }).click();
  await expect(period).toHaveText('Selected period: 2026-07-03 – 2026-07-08');
  await expect(start).toHaveAttribute('aria-valuenow', '1');
  await expect(end).toHaveAttribute('aria-valuenow', '3');
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('Other');
  await page.locator('.economy-item').filter({ hasText: "Gemcutter's Prism" }).click();
  await expect(period).toHaveText('Selected period: 2026-07-05 – 2026-07-06');
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('Standard');
  await page.locator('.economy-item').filter({ hasText: "Gemcutter's Prism" }).click();
  await expect(period).toHaveText('Selected period: 2026-07-03 – 2026-07-08');
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('archive:Ancestors');
  await page.locator('.economy-item').filter({ hasText: "Gemcutter's Prism" }).click();
  await expect(period).toHaveText('Selected period: 2026-07-04 – 2026-07-08');
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('Standard');
  await page.locator('.economy-item').filter({ hasText: "Gemcutter's Prism" }).click();
  await expect(period).toHaveText('Selected period: 2026-07-03 – 2026-07-08');
  await end.press('Enter');
  await page.getByRole('navigation', { name: 'Item breadcrumb', exact: true }).getByRole('button', { name: 'Currency', exact: true }).click();
  await page.locator('.economy-item').filter({ hasText: 'Vaal Orb' }).click();
  await expect(period).toHaveText('Selected period: 2026-07-02 – 2026-07-10');
});

test('overview graph drag selects ordered periods and captures releases beyond either edge', async ({ page }) => {
  await service(page);
  const points = Array.from({ length: 11 }, (_, index) => ({
    date: `2026-07-${String(index + 1).padStart(2, '0')}`,
    price: String(index + 1), currency: 'Chaos Orb', granularity: 'daily',
  }));
  await page.route('**/api/v1/economy/history?**', (route) => route.fulfill({
    json: { points, coverage: { start: points[0]!.date, end: points.at(-1)!.date } },
  }));
  await page.goto('/');
  await page.locator('.economy-item').first().click();
  const graph = page.locator('.history-brush');
  const start = page.getByRole('slider', { name: 'Period start', exact: true });
  const end = page.getByRole('slider', { name: 'Period end', exact: true });
  for (const [from, to, expectedStart, expectedEnd] of [
    [.2, .7, 2, 7], [.8, .3, 3, 8], [.4, -.1, 0, 4], [.6, 1.1, 6, 10], [.8, .1, 1, 8], [.4, .4, 4, 4],
  ]) {
    await graph.scrollIntoViewIfNeeded();
    const bounds = (await graph.boundingBox())!;
    await page.mouse.move(bounds.x + bounds.width * from!, bounds.y + bounds.height / 2);
    await page.mouse.down();
    await page.mouse.move(bounds.x + bounds.width * to!, bounds.y + bounds.height + 30, { steps: 8 });
    await page.mouse.up();
    await expect(start).toHaveAttribute('aria-valuenow', String(expectedStart));
    await expect(end).toHaveAttribute('aria-valuenow', String(expectedEnd));
    await expect(page.getByText(`History data (${expectedEnd! - expectedStart! + 1} observations)`, { exact: true })).toBeVisible();
  }
  await end.press('Enter');
  await page.setViewportSize({ width: 390, height: 800 });
  await graph.scrollIntoViewIfNeeded();
  const bounds = (await graph.boundingBox())!;
  const touch = await page.context().newCDPSession(page);
  await touch.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: bounds.x + bounds.width * .7, y: bounds.y + bounds.height / 2 }] });
  await touch.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: bounds.x - 12, y: bounds.y + bounds.height + 20 }] });
  await touch.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
  await touch.detach();
  await expect(start).toHaveAttribute('aria-valuenow', '0');
  await expect(end).toHaveAttribute('aria-valuenow', '7');
  await end.press('Enter');
  await expect(end).toHaveAttribute('aria-valuenow', '10');
});

test('history hover shows a guide and saved price at the nearest day in desktop and mobile layouts', async ({ page }) => {
  await service(page);
  await page.goto('/');
  await page.locator('.economy-item').first().click();
  const chart = page.getByRole('img', { name: 'Saved price history in Chaos Orb', exact: true });
  const tooltip = page.getByRole('tooltip');
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 800 });
    await chart.scrollIntoViewIfNeeded();
    const bounds = await chart.boundingBox();
    await page.mouse.move(bounds!.x + bounds!.width * .02, bounds!.y + bounds!.height / 2);
    await expect(tooltip).toHaveText('2026-07-15: 1 Chaos Orb (daily)');
    await expect(page.locator('.history-guide')).toHaveAttribute('d', 'M16,0 V200');
    await expect(page.locator('.history-plot circle')).toHaveAttribute('cx', '16');
    await expect(page.locator('.history-plot circle')).toHaveAttribute('r', '4');
    await page.mouse.move(bounds!.x + bounds!.width * .98, bounds!.y + bounds!.height / 2);
    await expect(tooltip).toHaveText('2026-07-16: 2 Chaos Orb (daily)');
    await expect(page.locator('.history-guide')).toHaveAttribute('d', 'M784,0 V200');
    await page.getByRole('slider', { name: 'Period start', exact: true }).press('ArrowRight');
    await expect(tooltip).toHaveCount(0);
    await chart.scrollIntoViewIfNeeded();
    await chart.hover();
    await expect(tooltip).toHaveText('2026-07-16: 2 Chaos Orb (daily)');
    await page.getByRole('slider', { name: 'Period start', exact: true }).press('Enter');
    await chart.hover();
    await page.mouse.move(0, 0);
    await expect(tooltip).toHaveCount(0);
    await expect(page.locator('.history-guide')).toHaveAttribute('d', '');
  }
});

test('price axes, point dots and compact volume graph follow the selected period', async ({ page }) => {
  await service(page);
  const points = [
    { date: '2026-07-14T12:15:43.123Z', price: '10000', volume: '20000', currency: 'Chaos Orb', granularity: 'snapshot' },
    { date: '2026-07-15T12:15:43.123Z', price: '20000', volume: null, currency: 'Chaos Orb', granularity: 'snapshot' },
    { date: '2026-07-16T12:15:43.123Z', price: '30000', volume: '80000', currency: 'Chaos Orb', granularity: 'snapshot' },
  ];
  await page.route('**/api/v1/economy/history?**', (route) => route.fulfill({
    json: { points, coverage: { start: points[0]!.date, end: points.at(-1)!.date } },
  }));
  await page.goto('/');
  await page.locator('.economy-item').first().click();
  await expect(page.locator('.history-period > p').first()).toHaveText('Selected period: 2026-07-14 – 2026-07-16');
  await expect(page.getByLabel('Price axis')).toHaveText('10 00015 00020 00025 00030 000');
  await expect(page.getByLabel('Day axis')).toContainText('Jul 14');
  expect((await page.locator('.history-dots').getAttribute('d'))?.match(/M/g)).toHaveLength(3);
  await expect(page.locator('.history-grid')).toHaveAttribute('d', /H784/);
  const volume = page.getByRole('region', { name: 'Volume history in Chaos Orb', exact: true });
  await expect(volume).toContainText('Max 80 000');
  const volumePlot = volume.getByRole('img');
  expect((await volumePlot.boundingBox())!.height).toBeLessThanOrEqual(60);
  expect((await volume.locator('path').getAttribute('d'))?.match(/M/g)).toHaveLength(2);
  const end = page.getByRole('slider', { name: 'Period end', exact: true });
  const start = page.getByRole('slider', { name: 'Period start', exact: true });
  await end.press('ArrowLeft');
  await expect(volume).toContainText('Max 20 000');
  await expect(page.getByLabel('Price axis')).not.toContainText('30');
  await start.press('ArrowRight');
  await expect(volume).toContainText('No recorded volume for this period');
  await expect(volumePlot).toHaveCount(0);
  await start.press('Enter');
  await expect(volume).toContainText('Max 80 000');
  await page.setViewportSize({ width: 390, height: 800 });
  await expect(page.getByLabel('Day axis')).toBeVisible();
  expect(await page.locator('.history-chart').evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
});

test('historical datasets and import manager do not select an archived league for the collector', async ({ page }) => {
  await service(page);
  const selections: string[] = [];
  page.on('request', (request) => { if (request.url().endsWith('/selection')) selections.push(request.url()); });
  await page.goto('/');
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('archive:Ancestors');
  await expect(page.locator('.app-header').getByRole('combobox', { name: 'As of', exact: true })).toHaveValue('2026-07-16');
  await expect(page.getByRole('button', { name: 'Stash', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('combobox', { name: 'League', exact: true })).toHaveValue('archive:Ancestors');
  await expect(page.getByRole('button', { name: 'Historical data', exact: true })).toHaveCount(0);
  await page.locator('.app-header').getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Settings', exact: true })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Current prices', exact: true })).toBeHidden();
  await expect(page.getByRole('region', { name: 'Historical data manager' })).toContainText('Allflame has not been published');
  await page.getByRole('button', { name: 'Import Mirage', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Browse Mirage', exact: true })).toBeVisible();
  expect(selections).toEqual([]);
});

test('shared dataset supports historical gem strategies and same-day materials without selecting archives for collection', async ({ page }) => {
  await service(page);
  const selections: string[] = [];
  page.on('request', (request) => {
    if (new URL(request.url()).pathname.endsWith('/selection')) selections.push(request.postDataJSON().id);
  });
  await page.route('**/api/v1/economy/overview?**', (route) => {
    const query = new URL(route.request().url()).searchParams;
    const category = query.get('category')!;
    const league = query.get('league')!;
    const date = query.get('date') ?? '';
    const overview = economyOverview(category, 'stash', league, date);
    if (league === 'Ancestors') overview.lines = category === 'SkillGem'
      ? gemOverview().gems.map((gem) => economyLine({
        name: gem.name, price: gem.chaos_value, variant: gem.variant,
        gem_level: gem.gem_level, gem_quality: gem.gem_quality, corrupted: gem.corrupted,
      }))
      : [economyLine({ name: "Gemcutter's Prism", price: date === '2026-07-16' ? '3' : '5' }),
        economyLine({ name: 'Vaal Orb', price: '4' })];
    return route.fulfill({ json: overview });
  });
  await page.goto('/');
  await expect(page.getByRole('combobox', { name: 'Economy dataset', exact: true })).toHaveCount(0);
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('archive:Ancestors');
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  await expect(page.locator('.app-header').getByRole('combobox', { name: 'As of', exact: true })).toHaveValue('2026-07-16');
  await expect(page.getByRole('combobox', { name: 'As of', exact: true })).toHaveCount(1);
  await expect(page.locator('.gem-table')).toContainText('CONF UNKNOWN');
  await expect(page.locator('.gem-table')).toContainText('N/A/N/A');
  await expect(page.getByRole('group', { name: 'Confidence filter', exact: true })).toHaveCount(0);
  await expect(page.locator('td.positive')).toHaveText('38.00c');
  await page.getByRole('button', { name: 'Add quality', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'GCP price in Chaos', exact: true })).toHaveAttribute('placeholder', '3');
  await expect(page.locator('td.positive')).toHaveText('0.00c');
  await page.locator('.app-header').getByRole('combobox', { name: 'As of', exact: true }).selectOption('2026-06-25');
  await expect(page.getByRole('textbox', { name: 'GCP price in Chaos', exact: true })).toHaveAttribute('placeholder', '5');
  await expect(page.locator('td.negative')).toHaveText('-40.00c');
  await page.getByRole('button', { name: 'Vaal corrupt', exact: true }).click();
  await expect(page.locator('.gem-table')).toContainText('N/A/N/A');
  await page.getByRole('button', { name: 'Current prices', exact: true }).click();
  await expect(page.locator('.app-header').getByRole('combobox', { name: 'As of', exact: true })).toHaveValue('2026-06-25');
  await page.getByRole('combobox', { name: 'As of', exact: true }).selectOption('2026-07-16');
  await expect(page.locator('.economy-content > .toolbar .muted')).toContainText('2026-07-16');
  expect(selections).toEqual([]);
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('Standard');
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  await expect(page.locator('.app-header').getByRole('combobox', { name: 'Gem price instant', exact: true })).toBeVisible();
  await expect(page.getByRole('group', { name: 'Confidence filter', exact: true })).toBeVisible();
  await expect(page.locator('.gem-table')).not.toContainText('CONF UNKNOWN');
});

test('archive Browse replaces the live date with an available historical date', async ({ page }) => {
  await service(page);
  await page.goto('/');
  await expect(page.locator('.economy-table tbody tr')).toHaveCount(2);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('button', { name: 'Browse Ancestors', exact: true }).click();
  await expect(page.getByRole('combobox', { name: 'League', exact: true })).toHaveValue('archive:Ancestors');
  await expect(page.locator('.app-header').getByRole('combobox', { name: 'As of', exact: true })).toHaveValue('2026-07-16');
  await expect(page.locator('.economy-table tbody tr')).toHaveCount(2);
  await expect(page.getByRole('region', { name: 'Settings', exact: true })).toHaveCount(0);
});

test('settings gear manages datasets from either tab and preserves price browsing context', async ({ page }) => {
  await service(page);
  await page.goto('/');
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('archive:Ancestors');
  await expect(page.getByRole('combobox', { name: 'As of', exact: true })).toHaveValue('2026-07-16');
  const search = page.getByRole('searchbox', { name: 'Search economy items', exact: true });
  await search.fill('Divine');
  const gear = page.locator('.app-header').getByRole('button', { name: 'Settings', exact: true });
  await expect(gear).toHaveAttribute('title', 'Settings');
  expect(await gear.evaluate((button) => button.previousElementSibling?.id)).toBe('refresh');
  await gear.click();
  await expect(gear).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  await expect(page.locator('settings-page dump-manager')).toHaveCount(1);
  await expect(page.locator('current-prices dump-manager')).toHaveCount(0);
  await expect(page.getByRole('combobox', { name: 'As of', exact: true })).toBeHidden();
  await expect(page.getByRole('button', { name: 'Browse Ancestors', exact: true })).toBeVisible();
  const marketRefreshes: string[] = [];
  page.on('request', (request) => {
    if (new URL(request.url()).pathname === '/api/v1/gem-analysis/refresh') marketRefreshes.push(request.url());
  });
  const catalogRefresh = page.waitForRequest((request) => {
    const url = new URL(request.url());
    return url.pathname === '/api/v1/economy/archives' && url.searchParams.get('refresh') === 'true';
  });
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await catalogRefresh;
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  expect(marketRefreshes).toEqual([]);
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Settings', exact: true })).toHaveCount(0);
  await expect(page.locator('dump-manager')).toHaveCount(0);
  await expect(gear).toHaveAttribute('aria-pressed', 'false');
  await gear.click();
  await expect(page.getByRole('region', { name: 'Historical data manager' })).toBeVisible();
  await page.getByRole('button', { name: 'Current prices', exact: true }).click();
  await expect(search).toHaveValue('Divine');
  await expect(page.getByRole('combobox', { name: 'League', exact: true })).toHaveValue('archive:Ancestors');
  await expect(page.getByRole('combobox', { name: 'As of', exact: true })).toHaveValue('2026-07-16');
});

test('settings polls active imports only while open and can retry a catalog failure', async ({ page }) => {
  await service(page);
  await page.clock.install();
  let requests = 0;
  await page.route('**/api/v1/economy/archives*', (route) => {
    requests++;
    return requests === 1
      ? route.fulfill({ status: 502, body: 'Catalog unavailable' })
      : route.fulfill({ json: [economyArchive('Mirage', 'importing')] });
  });
  await page.goto('/');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('Unable to load archive status');
  await page.getByRole('button', { name: 'Discover dumps', exact: true }).click();
  await expect(page.locator('.archive-entry')).toContainText('importing');
  await expect(page.getByRole('button', { name: 'Discover dumps', exact: true })).toBeEnabled();
  expect(requests).toBe(2);
  await page.clock.fastForward(3100);
  await expect.poll(() => requests).toBe(3);
  await expect(page.getByRole('button', { name: 'Discover dumps', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Current prices', exact: true }).click();
  await page.clock.fastForward(10000);
  expect(requests).toBe(3);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect.poll(() => requests).toBe(4);
});

test('routed pages support deep links, reload and browser history while preserving gem state', async ({ page }) => {
  await service(page);
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/#/settings');
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  await expect(page.locator('au-viewport > settings-page')).toHaveCount(1);
  await page.reload();
  await expect(page.getByRole('button', { name: 'Browse Ancestors', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  await expect(page).toHaveURL(/#\/gems$/);
  await page.getByRole('searchbox', { name: 'Search gems', exact: true }).fill('fire');
  await page.getByRole('combobox', { name: 'Gem price instant', exact: true }).selectOption('1');
  await expect(page.locator('.gem-table tbody tr')).toHaveCount(1);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page).toHaveURL(/#\/settings$/);
  await expect(page.locator('gem-flipping')).toHaveCount(0);
  await page.goBack();
  await expect(page).toHaveURL(/#\/gems$/);
  await expect(page.getByRole('searchbox', { name: 'Search gems', exact: true })).toHaveValue('fire');
  await expect(page.getByRole('combobox', { name: 'Gem price instant', exact: true })).toHaveValue('1');
  await page.goBack();
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  await page.goForward();
  await expect(page.getByRole('region', { name: 'Gem flipping', exact: true })).toBeVisible();
  await page.goForward();
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test('price details and selected graph period survive route destruction and a direct price URL reloads', async ({ page }) => {
  await service(page);
  await page.goto('/#/prices');
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('archive:Ancestors');
  await expect(page.getByRole('combobox', { name: 'As of', exact: true })).toHaveValue('2026-07-16');
  await page.locator('.economy-table tbody tr').first().getByRole('button').click();
  await page.getByRole('slider', { name: 'Period start', exact: true }).press('ArrowRight');
  const selected = await page.locator('.history-period > p').first().innerText();
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  await expect(page.locator('current-prices')).toHaveCount(0);
  await page.goBack();
  await expect(page.getByRole('navigation', { name: 'Item breadcrumb', exact: true })).toBeVisible();
  await expect(page.locator('.history-period > p').first()).toHaveText(selected);
  await expect(page.getByRole('combobox', { name: 'League', exact: true })).toHaveValue('archive:Ancestors');
  await expect(page.getByRole('combobox', { name: 'As of', exact: true })).toHaveValue('2026-07-16');
  await page.reload();
  await expect(page).toHaveURL(/#\/prices$/);
  await expect(page.getByRole('region', { name: 'Current prices', exact: true })).toBeVisible();
  await expect(page.locator('au-viewport > current-prices')).toHaveCount(1);
});

test('direct gem links load the shared datasets and price refresh preserves pinned gem state', async ({ page }) => {
  await service(page);
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/#/gems');
  await expect(page.getByRole('combobox', { name: 'League', exact: true })
    .locator('option[value="archive:Ancestors"]')).toHaveCount(1);
  await page.getByRole('searchbox', { name: 'Search gems', exact: true }).fill('fire');
  await page.getByRole('combobox', { name: 'Gem price instant', exact: true }).selectOption('1');
  await expect(page.locator('.gem-table tbody tr')).toHaveCount(1);
  await page.getByRole('button', { name: 'Current prices', exact: true }).click();
  await expect(page.locator('.economy-table tbody tr')).toHaveCount(2);
  const capture = page.waitForRequest((request) =>
    new URL(request.url()).pathname === '/api/v1/gem-analysis/refresh' && request.method() === 'POST');
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await capture;
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  const saved = page.waitForRequest((request) => {
    const url = new URL(request.url());
    return url.pathname === '/api/v1/gem-analysis' && url.searchParams.get('snapshot') === '1';
  });
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  await saved;
  await expect(page.getByRole('searchbox', { name: 'Search gems', exact: true })).toHaveValue('fire');
  await expect(page.getByRole('combobox', { name: 'Gem price instant', exact: true })).toHaveValue('1');
  await expect(page.locator('.gem-table tbody tr')).toHaveCount(1);
  expect(errors).toEqual([]);
});

test('main tabs, league, refresh and settings share one row without a branding header', async ({ page }) => {
  await service(page);
  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  await expect(page.getByText('Faustus', { exact: true })).toHaveCount(0);
  const header = page.locator('.app-header');
  await expect(header.getByRole('navigation', { name: 'Market pages' })).toBeVisible();
  for (const width of [1280, 720, 390]) {
    await page.setViewportSize({ width, height: 800 });
    const centers = await header.locator('nav button, select, #refresh, .settings-button').evaluateAll((elements) =>
      elements.map((element) => {
        const bounds = element.getBoundingClientRect();
        return bounds.y + bounds.height / 2;
      }));
    expect(centers).toHaveLength(6);
    expect(Math.max(...centers) - Math.min(...centers)).toBeLessThanOrEqual(1);
  }
  const capture = page.waitForRequest((request) =>
    new URL(request.url()).pathname === '/api/v1/gem-analysis/refresh' && request.method() === 'POST');
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  expect(new URL((await capture).url()).searchParams.get('league')).toBe('Standard');
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
});

test('web UI works without Tauri: prices, all gem strategies, search, Lens and history', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await service(page);
  await page.goto('/');
  await expect(page.getByRole('button', { name: "Gemcutter's Prism", exact: true })).toBeVisible();
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
  await page.getByRole('button', { name: 'Vaal corrupt', exact: true }).click();
  await expect(page.getByText('EV = expected value.', { exact: false })).toHaveCount(0);
  await expect(page.getByRole('button', { name: /Vaal \+/ })).toHaveCount(0);
  await expect(page.locator('td.negative > .weighted-return > .metric-value')).toHaveText('-14.50c');
  await expect(page.locator('.gem-table > table > thead th')).toHaveText(['Gem', 'Type', 'Buy', 'EV', 'Net v', 'ROI', 'Max value', 'Listings']);
  await expect(page.locator('.gem-table > table > tbody > tr > td').nth(6)).toHaveText('140.00c');
  const ev = page.locator('.weighted-return').nth(0);
  const net = page.locator('td.negative .weighted-return');
  const roi = page.locator('.weighted-return').nth(2);
  for (const metric of [ev, net, roi]) {
    const tooltip = metric.getByRole('tooltip');
    await expect(tooltip).toBeHidden();
    await metric.hover();
    await expect(tooltip).toBeVisible();
    const table = tooltip.getByRole('table', { name: 'Expected value breakdown' });
    await expect(table.locator('thead th')).toHaveText(['Outcome', 'Valued variant', 'Listings', 'Chance', 'Resale', 'Chance x resale']);
    await expect(table.locator('tbody tr')).toHaveCount(4);
    await expect(table.locator('tbody tr > td:nth-child(3)')).toHaveText(['12', '12', '12', '12']);
    await expect(table.locator('tbody tr').nth(0).locator('td')).toHaveText([
      '+1 level', 'Lv21 Q20 corrupted', '12', '12.5%', '140.00c', '17.50c',
    ]);
    await expect(table.locator('tbody tr').nth(2).locator('td')).toHaveText([
      'Max level corrupted (including releveled -1)', 'Lv20 Q20 corrupted', '12', '62.5%', '80.00c', '50.00c',
    ]);
    await expect(table.locator('tbody tr').nth(3)).toContainText('Estimate: lowest listed in bucket');
    await expect(table.locator('tbody tr').nth(3).locator('td').nth(2)).toHaveText('12');
    await expect(table.locator('tfoot tr').nth(0).locator('td')).toHaveText(['100.0%', '', '87.50c']);
    await expect(table.locator('tfoot tr').nth(2).locator('td')).toHaveText(['87.50c - 102.00c', '-14.50c']);
    await expect(table.locator('tfoot tr').nth(3).locator('td')).toHaveText(['-14.50c / 102.00c x 100', '-14.2%']);
    await metric.focus();
    await page.mouse.move(0, 0);
    await expect(tooltip).toBeVisible();
    await metric.blur();
    await expect(tooltip).toBeHidden();
  }
  await page.getByRole('button', { name: 'EV', exact: true }).click();
  await expect(page.getByRole('columnheader', { name: 'EV v', exact: true })).toHaveAttribute('aria-sort', 'descending');
  await page.getByRole('button', { name: 'EV v', exact: true }).click();
  await expect(page.getByRole('columnheader', { name: 'EV ^', exact: true })).toHaveAttribute('aria-sort', 'ascending');
  await page.setViewportSize({ width: 390, height: 800 });
  await ev.focus();
  const tooltipBounds = await ev.getByRole('tooltip').boundingBox();
  if (!tooltipBounds) throw new Error('Expected visible EV table');
  expect(tooltipBounds.x).toBeGreaterThanOrEqual(0);
  expect(tooltipBounds.x + tooltipBounds.width).toBeLessThanOrEqual(390);
  await ev.blur();
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.getByRole('button', { name: 'Max value', exact: true }).click();
  await expect(page.getByRole('columnheader', { name: 'Max value v', exact: true })).toHaveAttribute('aria-sort', 'descending');
  await page.getByRole('searchbox', { name: 'Search gems' }).fill('unmatched');
  await expect(page.getByRole('button', { name: 'Fireball', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Clear gem search' }).click();
  await page.getByRole('button', { name: 'Fireball', exact: true }).click();
  await page.getByRole('button', { name: 'Level flip', exact: true }).click();
  await expect(page.getByRole('columnheader', { name: /^Sell/ })).toBeVisible();
  await expect(page.getByRole('columnheader', { name: /Max value/ })).toHaveCount(0);
  await expect(page.getByRole('searchbox', { name: 'Search gems' })).toHaveValue('Fireball');
  await expect(page.getByRole('button', { name: 'GGG history', exact: true })).toHaveCount(0);
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

test('editable material prices update profits, validate input, and reset to snapshot prices', async ({ page }) => {
  await service(page);
  await page.goto('/');
  await page.getByRole('button', { name: 'Gem flipping', exact: true }).click();
  const gcp = page.getByRole('textbox', { name: 'GCP price in Chaos', exact: true });
  const vaal = page.getByRole('textbox', { name: 'Vaal price in Chaos', exact: true });
  await expect(gcp).toHaveAttribute('placeholder', '1');
  await expect(vaal).toHaveAttribute('placeholder', '2');
  await page.getByRole('button', { name: 'Add quality', exact: true }).click();
  await gcp.fill('1.5');
  await expect(page.locator('td.positive')).toHaveText('30.00c');
  await gcp.fill('-1');
  await expect(gcp).toHaveAttribute('aria-invalid', 'true');
  await expect(page.getByRole('alert')).toContainText('non-negative');
  await expect(page.locator('.gem-table tbody tr')).toHaveCount(0);
  await gcp.fill('0');
  await expect(page.locator('td.positive')).toHaveText('60.00c');
  await vaal.fill('3');
  await page.getByRole('button', { name: 'Vaal corrupt', exact: true }).click();
  await expect(page.locator('td.negative > .weighted-return > .metric-value')).toHaveText('-15.50c');
  await page.getByRole('combobox', { name: 'Gem price instant' }).selectOption('1');
  await expect(vaal).toHaveValue('3');
  await expect(page.locator('td.negative > .weighted-return > .metric-value')).toHaveText('-9.25c');
  await expect(page.getByRole('button', { name: 'Use automatic material prices', exact: true })).toHaveCount(0);
  await gcp.fill('');
  await vaal.fill('');
  await expect(vaal).toHaveValue('');
  await expect(vaal).toHaveAttribute('placeholder', '0.5');
  await expect(page.locator('td.negative > .weighted-return > .metric-value')).toHaveText('-6.75c');
  await expect(page.getByRole('button', { name: 'Use automatic material prices', exact: true })).toHaveCount(0);
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
  await expect(page.getByRole('textbox', { name: 'GCP price in Chaos', exact: true })).toHaveAttribute('placeholder', '0.5');
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  await expect(instant).toHaveValue('1');
  await expect(page.locator('td.positive')).toHaveText('20.00c');
  await page.getByRole('button', { name: 'Current prices', exact: true }).click();
  await expect(page.getByRole('row').filter({ hasText: "Gemcutter's Prism" }).locator('td').nth(1)).toHaveText('1Chaos Orb');
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
      await expect(page.locator('.economy-content > .toolbar .muted')).toContainText(`Updated ${localTimestamp}`);
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

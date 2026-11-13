import { expect, test, type Page } from '@playwright/test';
import { status } from '../src/testing/fixtures';
import { analysisPeriod, arbitrageAnalysis, flipAnalysis, flipSparklines, gggCoverage, gggHistory, gggJob, gggOverview } from '../src/testing/historical-fixtures';
import type { FlipRow, GggImportJob } from '../src/core/api-types';
import { localDate, localInput } from '../src/core/historical';

async function service(page: Page, options: { failImport?: boolean; failMarkets?: boolean; completeImport?: boolean; many?: boolean; confidenceRows?: boolean } = {}) {
  let job: GggImportJob | null = null;
  let polls = 0;
  const calls: { path: string; query: URLSearchParams; body: unknown }[] = [];
  await page.route('**/api/v1/**', async (route) => {
    const request = route.request(), url = new URL(request.url()), path = url.pathname;
    calls.push({ path, query: url.searchParams, body: request.postData() ? request.postDataJSON() : undefined });
    const start = Number(url.searchParams.get('start')), end = Number(url.searchParams.get('end'));
    if (path === '/api/v1/leagues') return route.fulfill({ json: [{ id: 'Standard', name: 'Standard' }, { id: 'Other', name: 'Other league' }] });
    if (path === '/api/v1/status') return route.fulfill({ json: status() });
    if (path === '/api/v1/selection') return route.fulfill({ json: { id: request.postDataJSON().id, name: 'Other league' } });
    if (path === '/api/v1/economy/leagues') return route.fulfill({ json: [
      { id: 'Ancestors', name: 'Ancestors', live: false, archived: true },
      { id: 'Mirage', name: 'Mirage', live: false, archived: true },
      ...(job?.status === 'completed' && job.league !== 'Standard'
        ? [{ id: job.league, name: job.league, live: false, archived: false }] : []),
    ] });
    if (path === '/api/v1/ggg/markets') return options.failMarkets
      ? route.fulfill({ status: 503, body: 'Saved GGG data unavailable' })
      : route.fulfill({ json: { ...gggOverview(Number(url.searchParams.get('hour'))), league: url.searchParams.get('league') } });
    if (path === '/api/v1/ggg/coverage') return route.fulfill({ json: gggCoverage(start, end) });
    if (path === '/api/v1/ggg/history') return route.fulfill({ json: gggHistory(start, end) });
    if (path === '/api/v1/ggg/import') {
      if (request.method() === 'POST') {
        if (options.failImport) return route.fulfill({ status: 502, body: 'GGG upstream rejected import' });
        job = { ...gggJob(), league: request.postDataJSON().league, warning: 'metadata_unknown=1/10; category_unknown=1/10' };
      } else if (job?.status === 'running') {
        polls++;
        job = { ...job, completed_hours: 2 + polls, status: options.completeImport ? 'completed' : 'running' };
      }
      return route.fulfill({ json: job });
    }
    if (path === '/api/v1/ggg/cancel') {
      job = gggJob('cancelled'); return route.fulfill({ json: job });
    }
    if (path === '/api/v1/analysis/period') return route.fulfill({ json: analysisPeriod(url.searchParams.get('league')!) });
    if (path === '/api/v1/analysis/sparklines') {
      const body = request.postDataJSON();
      expect(body.rows.length).toBeLessThanOrEqual(100);
      return route.fulfill({ json: flipSparklines(body.rows, body.start, body.end) });
    }
    if (path === '/api/v1/analysis/flips') {
      const result = flipAnalysis(start, end);
      if (options.confidenceRows) result.rows.push({
        ...result.rows[1]!, id: 'low-oil', name: 'Low confidence oil', start_confidence: 'Low', end_confidence: 'Low',
      });
      if (options.many) result.rows = Array.from({ length: 205 }, (_, i) => ({
        ...result.rows[0]!, id: `row-${i}`, name: `Item ${String(i).padStart(3, '0')}`,
      }));
      const search = url.searchParams.get('search')?.toLowerCase() ?? '', category = url.searchParams.get('category'), venue = url.searchParams.get('venue');
      const field = url.searchParams.get('sort')?.startsWith('value') ? 'growth' : 'growth_percent';
      const direction = url.searchParams.get('sort')?.endsWith('asc') ? 1 : -1;
      const minimumConfidence = url.searchParams.get('min_confidence');
      const confidenceRanks: Record<string, number> = { Low: 1, Medium: 2, High: 3 };
      const confidenceRank = (label?: string | null) => label ? confidenceRanks[label] ?? 0 : 0;
      const rows = result.rows.filter((row) => row.name.toLowerCase().includes(search)
        && (!category || row.category === category) && (!venue || row.venue === venue)
        && (!minimumConfidence || row.source === 'GGG Exchange'
          || [row.start_confidence, row.end_confidence].some(label => confidenceRank(label) >= confidenceRank(minimumConfidence))))
        .sort((a, b) => direction * (Number(a[field]) - Number(b[field])) || a.name.localeCompare(b.name));
      const offset = Number(url.searchParams.get('offset'));
      return route.fulfill({ json: { ...result, rows: rows.slice(offset, offset + 100), total: rows.length, offset,
        league: url.searchParams.get('league') } });
    }
    if (path === '/api/v1/analysis/arbitrage') return route.fulfill({ json: arbitrageAnalysis(Number(url.searchParams.get('hour'))) });
    if (path === '/api/v1/economy/history') return route.fulfill({ json: {
      points: [{ date: '2026-11-10', price: '10', currency: 'Chaos Orb', granularity: 'daily' }],
      coverage: { start: '2026-11-10', end: '2026-11-10' },
    } });
    throw new Error(`Unexpected historical API request: ${path}`);
  });
  return calls;
}

test('exchange direct link, saved pair history, import progress/cancel, route persistence and back/reload', async ({ page }) => {
  const calls = await service(page);
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/#/exchange');
  await expect(page.getByRole('heading', { name: 'Currency Exchange', exact: true })).toBeVisible();
  await expect(page.getByLabel('As of', { exact: true })).toBeHidden();
  await expect(page.getByRole('table', { name: 'GGG native pairs' })).toContainText('0.005');
  expect(calls.some((c) => c.path.endsWith('/import') && c.body)).toBe(false);
  await page.getByLabel('Search exchange').fill('Divine');
  await page.getByRole('button', { name: 'Chaos Orb / Divine Orb', exact: true }).click();
  await expect(page.getByRole('img', { name: 'Hourly average B per A; missing hours are gaps' })).toBeVisible();
  await expect(page.getByRole('table', { name: 'Pair history observations' }).locator('tbody tr')).toHaveCount(2);
  await expect(page.locator('.ggg-chart path')).toHaveCount(2);
  await page.getByRole('button', { name: 'Fetch missing hours', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'GGG PC import running:' })).toContainText('GGG PC import running: 2 / 24');
  await expect(page.getByRole('progressbar', { name: 'GGG PC import hour coverage' })).toHaveAttribute('value', '2');
  await expect(page.getByRole('alert')).toContainText('metadata_unknown');
  await expect(page.getByRole('status').filter({ hasText: 'GGG PC import running:' })).toContainText('GGG PC import running: 3 / 24');
  await page.getByRole('button', { name: 'Cancel import', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Import cancelled' })).toBeVisible();
  const hour = await page.getByLabel('Exchange hour').inputValue();
  await page.getByRole('button', { name: 'Flip analysis', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Flip analysis', exact: true })).toBeVisible();
  await page.goBack();
  await expect(page.getByLabel('Search exchange')).toHaveValue('Divine');
  await expect(page.getByLabel('Exchange hour')).toHaveValue(hour);
  await expect(page.getByRole('article', { name: 'Pair history' })).toBeVisible();
  await page.reload();
  await expect(page.getByRole('table', { name: 'GGG native pairs' })).toContainText('Divine Orb');
  expect(errors).toEqual([]);
});

test('exchange completed import reloads saved hours; visible import and saved-read errors', async ({ page }) => {
  const calls = await service(page, { completeImport: true });
  await page.goto('/#/exchange');
  await page.getByLabel('GGG league override').fill('Retained League');
  await page.getByRole('button', { name: 'Fetch missing hours', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Import completed' })).toBeVisible();
  await expect(page.getByRole('combobox', { name: 'League', exact: true }).locator('option[value="archive:Retained League"]')).toHaveCount(1);
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('archive:Retained League');
  await page.getByRole('button', { name: 'Flip analysis', exact: true }).click();
  await expect.poll(() => calls.filter((c) => c.path.endsWith('/analysis/flips')).at(-1)?.query.get('league')).toBe('Retained League');
  await page.getByRole('button', { name: 'Triangular arbitrage', exact: true }).click();
  await expect.poll(() => calls.filter((c) => c.path.endsWith('/analysis/arbitrage')).at(-1)?.query.get('league')).toBe('Retained League');
  await page.getByRole('button', { name: 'Currency Exchange', exact: true }).click();
  expect(calls.filter((c) => c.path.endsWith('/ggg/markets')).length).toBeGreaterThan(1);
  await page.unrouteAll({ behavior: 'wait' });
  await service(page, { failImport: true });
  await page.reload();
  await page.getByRole('button', { name: 'Fetch missing hours', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('GGG upstream rejected import');
  await page.unrouteAll({ behavior: 'wait' });
  await service(page, { failMarkets: true });
  await page.reload();
  await expect(page.getByRole('alert')).toContainText('Saved GGG data unavailable');
});

test('exchange custom retained league and archived shared selection use explicit hourly requests', async ({ page }) => {
  const calls = await service(page);
  await page.goto('/#/exchange');
  await expect(page.getByRole('table', { name: 'GGG native pairs' })).toContainText('Chaos Orb');
  await page.getByLabel('GGG league override').fill('Retained League');
  await page.getByRole('button', { name: 'Load saved hours', exact: true }).click();
  await expect.poll(() => calls.filter((c) => c.path.endsWith('/ggg/markets')).at(-1)?.query.get('league')).toBe('Retained League');
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('archive:Ancestors');
  await expect(page.getByLabel('GGG league override')).toHaveValue('');
  await expect.poll(() => calls.filter((c) => c.path.endsWith('/ggg/markets')).at(-1)?.query.get('league')).toBe('Ancestors');
  expect(calls.some((c) => c.path.endsWith('/import') && c.body)).toBe(false);
});

test('flips exact filters, min/max validation, numeric sorts, alternatives, exclusions and shared league', async ({ page }) => {
  const calls = await service(page);
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/#/flips');
  const table = page.getByRole('table', { name: 'Ranked flips', exact: true });
  await expect(table.locator('tbody > tr').first()).toContainText('Golden Oil');
  await expect(table.getByRole('columnheader', { name: 'Item volume (start → end)', exact: true })).toBeVisible();
  await expect(table.locator('tbody > tr').first().locator('td').nth(5)).toHaveText('Unknown');
  await expect(table.locator('tbody > tr').filter({ has: page.getByRole('button', { name: 'Divine Orb', exact: true }) }).locator('td').nth(5)).toHaveText('50 → 60');
  await page.getByLabel('Minimum starting price (Chaos)', { exact: true }).fill('100');
  await page.getByLabel('Maximum starting price (Chaos)', { exact: true }).fill('10');
  await page.getByRole('button', { name: 'Analyze flips', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('must not exceed');
  await page.getByLabel('Maximum starting price (Chaos)', { exact: true }).fill('200');
  await page.getByLabel('Minimum exchange item volume', { exact: true }).fill('50');
  await page.getByRole('button', { name: 'Analyze flips', exact: true }).click();
  await expect(table).toBeVisible();
  const query = calls.filter((c) => c.path.endsWith('/analysis/flips')).at(-1)!.query;
  expect(query.get('min_price')).toBe('100'); expect(query.get('max_price')).toBe('200'); expect(query.get('min_volume')).toBe('50');
  expect(Number(query.get('end')) - Number(query.get('start'))).toBe(86400);
  await page.getByLabel('Flip sort').selectOption('value-desc');
  await expect(table.locator('tbody > tr').first()).toContainText('Divine Orb');
  await page.getByRole('button', { name: 'Divine Orb', exact: true }).click();
  await expect(page.getByRole('table', { name: 'Divine Orb pair alternatives', exact: true })).toContainText('Selected');
  await expect(page.getByRole('img', { name: 'Flip native pair hourly history; missing hours are gaps' })).toBeVisible();
  const nativePaths = page.getByRole('img', { name: 'Flip native pair hourly history; missing hours are gaps' }).locator('path');
  await expect(nativePaths).toHaveCount(2);
  expect(await nativePaths.evaluateAll((paths) =>
    paths.every((path) => path.getAttribute('d')?.startsWith('M') && (path as SVGPathElement).getTotalLength() > 0))).toBe(true);
  await expect(page.getByText('Actual observations:', { exact: false })).toBeVisible();
  await page.getByText('Excluded items (1; showing up to 100)', { exact: true }).click();
  await expect(page.getByText('Silver Oil: Missing end observation', { exact: true })).toBeVisible();
  await page.getByLabel('Flip category').selectOption('Oils');
  await expect(table).not.toContainText('Divine Orb');
  await page.getByRole('button', { name: 'Golden Oil', exact: true }).click();
  await expect(page.getByText('Original native price: 10 → 15 · Chaos Orb', { exact: true })).toBeVisible();
  await expect(page.getByRole('table', { name: 'Golden Oil supporting history' })).toContainText('2026-11-10 (UTC day)');
  await page.getByLabel('Flip venue').selectOption('trade');
  await expect(table).toContainText('Golden Oil');
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('Other');
  await expect.poll(() => calls.filter((c) => c.path.endsWith('/analysis/flips')).at(-1)?.query.get('league')).toBe('Other league');
  await page.getByRole('button', { name: 'Triangular arbitrage', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Triangular arbitrage', exact: true })).toBeVisible();
  await page.goBack();
  await expect(page.getByLabel('Flip sort')).toHaveValue('value-desc');
  await expect(page.getByLabel('Minimum starting price (Chaos)', { exact: true })).toHaveValue('100');
  await expect(page.getByLabel('Flip category')).toHaveValue('Oils');
  expect(errors).toEqual([]);
});

test('flip explanations and coverage notices stay in the accessible info disclosure while errors remain visible', async ({ page }) => {
  await service(page);
  await page.goto('/#/flips');
  await expect(page.getByRole('table', { name: 'Ranked flips', exact: true })).toBeVisible();
  const info = page.getByRole('button', { name: 'Flip analysis information', exact: true });
  const panel = page.locator('.analysis-info-content');
  await expect(panel).not.toBeVisible();
  await expect(page.getByLabel('Selected period', { exact: true })).toHaveCount(0);
  await expect(page.getByText('Lines mark actual recorded hours / UTC dataset days; item coverage can differ.', { exact: true })).not.toBeVisible();
  await expect(page.locator('[aria-label="Period coverage notices"]')).not.toBeVisible();
  await expect(page.locator('[aria-label="Flip coverage notices"]')).not.toBeVisible();
  await info.focus();
  await page.keyboard.press('Enter');
  await expect(panel).toBeVisible();
  await expect(panel).toContainText('Historical endpoint growth in Chaos');
  await expect(panel).toContainText('Volume filters GGG item-unit observations only');
  await expect(panel).toContainText('Lines mark actual recorded hours');
  await expect(panel).toContainText('Archive-derived bounds; official league dates unavailable.');
  await expect(panel).toContainText('Only saved in-period starts and selected-end observations are ranked.');
  await page.keyboard.press('Tab');
  await expect(panel).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(panel).not.toBeVisible();
  await expect(info).toBeFocused();
  await page.setViewportSize({ width: 390, height: 800 });
  await info.click();
  await expect(panel).toBeVisible();
  const box = (await panel.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(390);
  await info.click();
  await expect(panel).not.toBeVisible();
  await page.getByLabel('Minimum starting price (Chaos)', { exact: true }).fill('100');
  await page.getByLabel('Maximum starting price (Chaos)', { exact: true }).fill('10');
  await page.getByRole('button', { name: 'Analyze flips', exact: true }).click();
  await expect(page.getByRole('alert')).toBeVisible();
  await expect(page.getByRole('alert')).toContainText('must not exceed');
  await expect(panel).not.toBeVisible();
});

test('flip native history inverts sub-unit ratios with their units and preserves lifeforce and gaps', async ({ page }) => {
  await service(page);
  await page.route('**/api/v1/ggg/history?**', async (route) => {
    const url = new URL(route.request().url());
    const start = Number(url.searchParams.get('start')), end = Number(url.searchParams.get('end'));
    const history = gggHistory(start, end);
    const market = history.points[0]!.market;
    const base = { ...market, a: { ...market.a, name: 'Chaos Orb' }, b: { ...market.b, name: 'The Chosen' } };
    history.points = [
      { hour: start, market: { ...base, volume_a: '0', volume_b: '0' } },
      { hour: start + 3600, market: { ...base, volume_a: '2', volume_b: '1' } },
      { hour: start + 7200, market: { ...base, volume_a: '4', volume_b: '1' } },
      { hour: end, market: { ...base, b: { ...base.b, name: 'Wild Crystallised Lifeforce' }, volume_a: '1', volume_b: '90' } },
    ];
    await route.fulfill({ json: history });
  });
  await page.goto('/#/flips');
  await page.getByRole('button', { name: 'Divine Orb', exact: true }).click();
  await page.getByText('Native history observations (4)', { exact: true }).click();
  await expect(page.getByText(/: 2 Chaos Orb per The Chosen$/)).toBeVisible();
  await expect(page.getByText(/: 4 Chaos Orb per The Chosen$/)).toBeVisible();
  await expect(page.getByText(/: - The Chosen per Chaos Orb$/)).toBeVisible();
  await expect(page.getByText(/: 90 Wild Crystallised Lifeforce per Chaos Orb$/)).toBeVisible();
  await expect(page.getByText(/: 0,5 The Chosen per Chaos Orb$/)).toHaveCount(0);
});

test('minimum confidence filters ninja observations only and persists across navigation', async ({ page }) => {
  const calls = await service(page, { confidenceRows: true });
  await page.goto('/#/flips');
  const confidence = page.getByLabel('Minimum poe.ninja confidence', { exact: true });
  const table = page.getByRole('table', { name: 'Ranked flips', exact: true });
  await expect(confidence).toHaveValue('');
  await expect(table).toContainText('Golden Oil');
  await expect(table).toContainText('Low confidence oil');
  await confidence.selectOption('Medium');
  await expect(table).not.toContainText('Low confidence oil');
  await expect(table).toContainText('Golden Oil');
  await expect(table).toContainText('Divine Orb');
  expect(calls.filter(c => c.path.endsWith('/analysis/flips')).at(-1)?.query.get('min_confidence')).toBe('Medium');
  await page.getByRole('button', { name: 'Currency Exchange', exact: true }).click();
  await page.getByRole('button', { name: 'Flip analysis', exact: true }).click();
  await expect(confidence).toHaveValue('Medium');
  await confidence.selectOption('High');
  await expect(table).not.toContainText('Low confidence oil');
  await expect(table).toContainText('Golden Oil');
  await confidence.selectOption('Low');
  await expect(table).toContainText('Low confidence oil');
  await confidence.selectOption('');
  await expect.poll(() => calls.filter(c => c.path.endsWith('/analysis/flips')).at(-1)?.query.has('min_confidence')).toBe(false);
});

test('strict start date defaults off, reranks on toggle and persists across navigation', async ({ page }) => {
  const calls = await service(page);
  await page.goto('/#/flips');
  const checkbox = page.getByRole('checkbox', { name: 'Strict start date', exact: true });
  await expect(checkbox).not.toBeChecked();
  await expect(page.getByRole('table', { name: 'Ranked flips', exact: true })).toContainText('Golden Oil');
  const start = await page.getByLabel('Flip start', { exact: true }).inputValue();
  const end = await page.getByLabel('Flip end', { exact: true }).inputValue();
  await checkbox.check();
  await expect.poll(() => calls.filter(c => c.path.endsWith('/analysis/flips')).at(-1)?.query.get('strict_start')).toBe('true');
  await page.getByRole('button', { name: 'Currency Exchange', exact: true }).click();
  await page.getByRole('button', { name: 'Flip analysis', exact: true }).click();
  await expect(checkbox).toBeChecked();
  await expect(page.getByLabel('Flip start', { exact: true })).toHaveValue(start);
  await expect(page.getByLabel('Flip end', { exact: true })).toHaveValue(end);
  await checkbox.uncheck();
  await expect.poll(() => calls.filter(c => c.path.endsWith('/analysis/flips')).at(-1)?.query.has('strict_start')).toBe(false);
});

test('flip rows display later actual starts without changing the selected period', async ({ page }) => {
  await service(page);
  let actualStart = '';
  await page.route('**/api/v1/analysis/flips?**', async (route) => {
    const url = new URL(route.request().url());
    const start = Number(url.searchParams.get('start')), end = Number(url.searchParams.get('end'));
    const result = flipAnalysis(start, end);
    const row = result.rows[0]!;
    actualStart = localDate(start + 3600);
    row.start_observed_at = new Date((start + 3600) * 1000).toISOString();
    row.alternatives[0]!.start_observed_at = row.start_observed_at;
    row.alternatives[0]!.end_observed_at = row.end_observed_at;
    result.rows[1]!.start_observed_at = '2026-11-10';
    await route.fulfill({ json: result });
  });
  await page.goto('/#/flips');
  const table = page.getByRole('table', { name: 'Ranked flips', exact: true });
  await expect(table).toContainText('Divine Orb');
  const selectedStart = await page.getByLabel('Flip start', { exact: true }).inputValue();
  const selectedEnd = await page.getByLabel('Flip end', { exact: true }).inputValue();
  const row = table.locator('tbody > tr').filter({ has: page.getByRole('button', { name: 'Divine Orb', exact: true }) });
  await expect(row.locator('td').nth(1)).toContainText(actualStart);
  expect(actualStart).not.toBe(selectedStart);
  await expect(table).toContainText('2026-11-10 (UTC day)');
  await page.getByRole('button', { name: 'Divine Orb', exact: true }).click();
  await expect(page.getByRole('table', { name: 'Divine Orb pair alternatives' })).toContainText(actualStart);
  await expect(page.getByLabel('Flip start', { exact: true })).toHaveValue(selectedStart);
  await expect(page.getByLabel('Flip end', { exact: true })).toHaveValue(selectedEnd);
});

test('flip date controls display ISO dates with working bounded native pickers and visible validation', async ({ page }) => {
  const calls = await service(page);
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/#/flips');
  await expect(page.getByRole('table', { name: 'Ranked flips', exact: true })).toBeVisible();
  for (const endpoint of ['start', 'end']) {
    const input = page.getByLabel(`Flip ${endpoint}`, { exact: true });
    await expect(input).toHaveAttribute('type', 'text');
    await expect(input).toHaveValue(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);
    const picker = page.locator('.native-date-picker').nth(endpoint === 'start' ? 0 : 1);
    await expect(picker).toHaveAttribute('min', /T\d{2}:\d{2}$/);
    await expect(picker).toHaveAttribute('max', /T\d{2}:\d{2}$/);
    await page.getByRole('button', { name: `Choose ${endpoint} date and time`, exact: true }).click();
    await page.keyboard.press('Escape');
    const value = await input.inputValue();
    const changed = localInput(new Date(value.replace(' ', 'T')).getTime() / 1000 + (endpoint === 'start' ? 3600 : -3600));
    await picker.evaluate((element, value) => {
      if (!(element instanceof HTMLInputElement)) throw new Error('Expected native date input');
      element.value = value;
      element.dispatchEvent(new Event('input', { bubbles: true }));
    }, changed);
    await expect(input).toHaveValue(changed.replace('T', ' '));
    await input.fill(value);
  }
  const start = page.getByLabel('Flip start', { exact: true });
  const requestCount = calls.filter((call) => call.path.endsWith('/analysis/flips')).length;
  await start.fill('2026-01-01 00:00');
  await page.getByRole('button', { name: 'Analyze flips', exact: true }).click();
  await expect(start).toHaveValue(localInput(analysisPeriod().start!).replace('T', ' '));
  await expect(page.getByRole('alert')).toHaveCount(0);
  expect(calls.filter((call) => call.path.endsWith('/analysis/flips'))).toHaveLength(requestCount + 1);
  await start.fill('2026-02-31 00:00');
  await page.getByRole('button', { name: 'Analyze flips', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('valid local date');
  await page.setViewportSize({ width: 390, height: 800 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  expect(errors).toEqual([]);
});

test('long flip names wrap on desktop and mobile while numeric columns stay single-line', async ({ page }) => {
  await service(page);
  const name = 'Claw Attacks deal 12% increased Damage with Hits and Ailments, Dagger Attacks deal 12% increased Damage with Hits and Ailments (8 passives, Large Cluster Jewel)';
  await page.route('**/api/v1/analysis/flips?**', (route) => {
    const url = new URL(route.request().url());
    const result = flipAnalysis(Number(url.searchParams.get('start')), Number(url.searchParams.get('end')));
    result.rows[0]!.name = name;
    return route.fulfill({ json: result });
  });
  await page.goto('/#/flips');
  const button = page.getByRole('button', { name, exact: true });
  await expect(button).toBeVisible();
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const layout = await button.evaluate((element) => {
      const style = getComputedStyle(element);
      const range = document.createRange();
      range.selectNodeContents(element);
      return {
        width: element.getBoundingClientRect().width,
        lines: range.getClientRects().length,
        overflow: element.scrollWidth > element.clientWidth,
        whiteSpace: style.whiteSpace,
      };
    });
    expect(layout.width).toBeLessThanOrEqual(Math.min(448, width * 0.6));
    expect(layout.lines).toBeGreaterThan(1);
    expect(layout.overflow).toBe(false);
    expect(layout.whiteSpace).toBe('normal');
    const cells = button.locator('xpath=ancestor::tr').locator('td');
    await expect(cells).toHaveCount(8);
    const numericCells = button.locator('xpath=ancestor::tr').locator('td:nth-child(n+2):nth-child(-n+6)');
    await expect(numericCells).toHaveCount(5);
    expect(await numericCells.evaluateAll((cells) => cells.every((cell) => getComputedStyle(cell).whiteSpace === 'nowrap'))).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
  await button.click();
  await expect(page.getByText('Selected pair:', { exact: false })).toBeVisible();
});

test('flip ranking renders before asynchronous charts, with provider confidence, gaps and responsive layout', async ({ page }) => {
  const calls = await service(page);
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  await page.route('**/api/v1/analysis/sparklines', async (route) => {
    await gate;
    const body = route.request().postDataJSON();
    await route.fulfill({ json: flipSparklines(body.rows, body.start, body.end) });
  });
  await page.goto('/#/flips');
  const table = page.getByRole('table', { name: 'Ranked flips', exact: true });
  await expect(table).toContainText('Divine Orb');
  await expect(table).toContainText('High → Low');
  await expect(table.locator('tbody > tr').filter({ hasText: 'Divine Orb' }).locator('td').nth(6)).toHaveText('—');
  await expect(page.getByRole('button', { name: 'Analyze flips', exact: true })).toBeEnabled();
  await expect(page.getByRole('status').filter({ hasText: 'Loading historical charts' })).toBeVisible();
  await expect(table.locator('.flip-sparkline')).toHaveCount(0);
  release();
  await expect(table.locator('.flip-sparkline')).toHaveCount(2);
  await expect(table.locator('.flip-sparkline path')).toHaveCount(4);
  expect(await table.locator('.flip-sparkline path').evaluateAll((paths) =>
    paths.every((path) => path.getAttribute('d')?.startsWith('M') && (path as SVGPathElement).getTotalLength() > 0))).toBe(true);
  await expect(table.getByRole('img').first()).toHaveAccessibleName(/Historical Chaos prices: 4 observations/);
  await expect(table.locator('.flip-chart-cell').first()).toHaveAttribute('title', /Unsampled fluctuations and gaps are not represented/);
  expect(await table.innerText()).not.toContain('Unsampled fluctuations and gaps are not represented.');
  await page.getByRole('button', { name: 'Flip analysis information', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Flip analysis information details', exact: true })).toContainText('Unsampled fluctuations and gaps are not represented.');
  await page.getByRole('button', { name: 'Flip analysis information', exact: true }).click();
  expect(calls.filter((call) => call.path.endsWith('/analysis/flips'))).toHaveLength(1);
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(table.locator('th')).toHaveCount(8);
    expect(await table.locator('.flip-sparkline').first().evaluate((element) => element.getBoundingClientRect().width)).toBe(120);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
});

test('two-day flip sparklines draw price paths and position isolated observations', async ({ page }) => {
  await service(page);
  await page.route('**/api/v1/analysis/period?**', (route) => {
    const period = analysisPeriod();
    period.start = period.end! - 2 * 86400;
    period.hours = [period.start, period.end!];
    period.days = [0, 1, 2].map((day) => new Date((period.start! + day * 86400) * 1000).toISOString().slice(0, 10));
    return route.fulfill({ json: period });
  });
  await page.route('**/api/v1/analysis/sparklines', (route) => {
    const { rows, start, end }: { rows: FlipRow[]; start: number; end: number } = route.request().postDataJSON();
    expect(end - start).toBe(2 * 86400);
    return route.fulfill({ json: { series: rows.map((row, index) => ({
      id: row.id, warnings: [], points: index === 0
        ? [{ time: start, price: '10' }, { time: start + 86400, price: '30' }, { time: end, price: '20' }]
        : [{ time: start + 86400, price: '15' }],
    })) } });
  });
  await page.goto('/#/flips');
  const charts = page.getByRole('table', { name: 'Ranked flips', exact: true }).locator('.flip-sparkline');
  await expect(charts).toHaveCount(2);
  const path = charts.first().locator('path');
  await expect(path).toHaveAttribute('d', 'M4,32 L60,4 L116,18');
  expect(await path.evaluate((element) => (element as SVGPathElement).getTotalLength())).toBeGreaterThan(112);
  const dot = charts.last().locator('circle');
  await expect(dot).toHaveAttribute('cx', '60');
  await expect(dot).toHaveAttribute('cy', '18');
});

test('flip chart errors are explicit, missing confidence is Unknown and absent prices never fabricate zero charts', async ({ page }) => {
  await service(page);
  await page.route('**/api/v1/analysis/flips?**', (route) => {
    const result = flipAnalysis();
    result.rows[1]!.start_confidence = null; result.rows[1]!.end_confidence = null;
    return route.fulfill({ json: result });
  });
  await page.route('**/api/v1/analysis/sparklines', (route) => route.fulfill({ status: 503, body: 'Chart storage unavailable' }));
  await page.goto('/#/flips');
  const table = page.getByRole('table', { name: 'Ranked flips', exact: true });
  await expect(table).toContainText('Unknown → Unknown');
  await expect(page.getByRole('alert')).toContainText('Historical charts unavailable');
  await expect(table).toContainText('Divine Orb');
  await page.route('**/api/v1/analysis/sparklines', (route) => {
    const rows: FlipRow[] = route.request().postDataJSON().rows;
    return route.fulfill({ json: { series: rows.map((row) => ({ id: row.id, points: [], warnings: ['No saved observations'] })) } });
  });
  await page.getByRole('button', { name: 'Analyze flips', exact: true }).click();
  await expect(table.locator('.flip-sparkline')).toHaveCount(0);
  await expect(table).toContainText('No historical Chaos price observations available.');
  await expect(table.locator('.flip-chart-cell').first()).toHaveAttribute('title', 'No saved observations');
  await expect(page.getByRole('alert')).toHaveCount(0);
});

test('search and league switches cancel late chart batches and only render the current displayed rows', async ({ page }) => {
  await service(page);
  const releases: (() => void)[] = [];
  const batches: { league: string; rows: FlipRow[] }[] = [];
  await page.route('**/api/v1/analysis/sparklines', async (route) => {
    const body = route.request().postDataJSON();
    batches.push(body);
    await new Promise<void>((resolve) => { releases.push(resolve); });
    const response = flipSparklines(body.rows, body.start, body.end);
    response.series.forEach((series) => { series.warnings = [`Chart batch ${batches.indexOf(body)}`]; });
    try { await route.fulfill({ json: response }); } catch { /* The browser may already have aborted this request. */ }
  });
  await page.goto('/#/flips');
  await expect.poll(() => batches.length).toBe(1);
  await page.getByLabel('Search flips').fill('Golden');
  await expect.poll(() => batches.length).toBe(2);
  expect(batches[1]!.rows.map((row) => row.id)).toEqual(['oil']);
  releases[0]!();
  const table = page.getByRole('table', { name: 'Ranked flips', exact: true });
  await expect(table.locator('.flip-sparkline')).toHaveCount(0);
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('Other');
  await expect.poll(() => batches.length).toBe(3);
  expect(batches[2]!.league).toBe('Other league');
  releases[1]!(); releases[2]!();
  await expect(table.locator('.flip-sparkline')).toHaveCount(1);
  await expect(table.locator('.flip-chart-cell')).toHaveAttribute('title', 'Chart batch 2');
});

test('flip period bounds follow league data, preserve custom navigation and support zoom, drag and keyboard commits', async ({ page }) => {
  const calls = await service(page);
  const errors: string[] = []; page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()); });
  await page.goto('/#/flips');
  await expect(page.getByRole('table', { name: 'Ranked flips', exact: true })).toBeVisible();
  expect(errors).toEqual([]);
  await expect(page.locator('.period-markers line')).toHaveCount(5);
  const start = page.getByLabel('Flip start'), end = page.getByLabel('Flip end');
  const initial = await start.inputValue();
  const svg = page.locator('period-band svg');
  const box = (await svg.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  const beforeZoom = await page.locator('.period-caption small').innerText();
  await page.mouse.wheel(0, -350);
  await expect(page.locator('.period-caption small')).not.toHaveText(beforeZoom);
  const beforeDrag = calls.filter((c) => c.path.endsWith('/analysis/flips')).length;
  await page.getByRole('button', { name: 'Reset zoom', exact: true }).click();
  const slider = page.getByRole('slider', { name: 'Period start', exact: true });
  await slider.focus(); await page.keyboard.press('ArrowRight');
  await expect(start).not.toHaveValue(initial);
  await expect.poll(() => calls.filter((c) => c.path.endsWith('/analysis/flips')).length).toBe(beforeDrag + 1);
  const selected = await start.inputValue();
  await page.getByRole('button', { name: 'Triangular arbitrage', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Triangular arbitrage', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Flip analysis', exact: true }).click();
  await expect(start).toHaveValue(selected);
  await page.getByRole('combobox', { name: 'League', exact: true }).selectOption('archive:Mirage');
  await expect.poll(() => calls.filter((c) => c.path.endsWith('/analysis/flips')).at(-1)?.query.get('league')).toBe('Mirage');
  expect((await start.inputValue()).slice(0, 7)).toBe('2026-03');
  expect((await end.inputValue()).slice(0, 7)).toBe('2026-03');
  const bounds = analysisPeriod('Mirage');
  const last = calls.filter((c) => c.path.endsWith('/analysis/flips')).at(-1)!.query;
  expect(Number(last.get('start'))).toBe(bounds.start); expect(Number(last.get('end'))).toBe(bounds.end);
  await page.getByLabel('Minimum starting price (Chaos)', { exact: true }).fill('1');
  await page.getByLabel('Maximum starting price (Chaos)', { exact: true }).fill('100');
  await page.getByLabel('Minimum exchange item volume', { exact: true }).fill('1');
  await page.getByRole('button', { name: 'Analyze flips', exact: true }).click();
  await expect(page.getByRole('table', { name: 'Ranked flips', exact: true })).toContainText('Golden Oil');
  await expect(page.getByRole('button', { name: 'Analyze flips', exact: true })).toBeEnabled();
  const dragBox = (await svg.boundingBox())!;
  const count = calls.filter((c) => c.path.endsWith('/analysis/flips')).length;
  await page.mouse.move(dragBox.x + 3, dragBox.y + dragBox.height / 2); await page.mouse.down();
  await page.mouse.move(dragBox.x + dragBox.width * 0.6, dragBox.y + dragBox.height / 2, { steps: 5 });
  expect(calls.filter((c) => c.path.endsWith('/analysis/flips'))).toHaveLength(count);
  await page.mouse.up();
  await expect.poll(() => calls.filter((c) => c.path.endsWith('/analysis/flips')).length).toBe(count + 1);
  await page.setViewportSize({ width: 390, height: 800 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  expect(errors).toEqual([]);
});

test('all-category flips use server pages, sort and search across the complete dataset', async ({ page }) => {
  const calls = await service(page, { many: true });
  await page.goto('/#/flips');
  const rows = page.getByRole('table', { name: 'Ranked flips', exact: true }).locator('tbody > tr');
  await expect(rows).toHaveCount(100);
  await page.getByRole('button', { name: 'Next 100 flips', exact: true }).click();
  await expect(page.getByText('Showing 101–200 of 205 matching items.', { exact: true })).toBeVisible();
  await expect(rows).toHaveCount(100);
  expect(calls.filter((c) => c.path.endsWith('/analysis/flips')).at(-1)?.query.get('offset')).toBe('100');
  await page.getByRole('button', { name: 'Next 100 flips', exact: true }).click();
  await expect(rows).toHaveCount(5);
  await expect(page.getByRole('button', { name: 'Next 100 flips', exact: true })).toBeDisabled();
  await page.getByLabel('Search flips').fill('Item 204');
  await expect(rows).toHaveCount(1); await expect(rows).toContainText('Item 204');
  const query = calls.filter((c) => c.path.endsWith('/analysis/flips')).at(-1)!.query;
  expect(query.get('search')).toBe('Item 204'); expect(query.get('offset')).toBe('0'); expect(query.get('limit')).toBe('100');
});

test('flip prices, growth, percentages and native alternatives use compact shared numeric formatting', async ({ page }) => {
  await service(page);
  const result = flipAnalysis(), row = result.rows[0]!;
  row.start_price = '94.23'; row.end_price = '12345.6789'; row.growth = '12251.4489';
  row.growth_percent = '8932.258064516129032258064516';
  row.native_start = '0.000123456789'; row.native_end = '94.23';
  Object.assign(row.alternatives[0]!, { native_start: row.native_start, native_end: row.native_end,
    start_price: row.start_price, end_price: row.end_price, growth: row.growth, growth_percent: row.growth_percent,
    start_volume: '12345.6789', end_volume: '15000' });
  result.rows = [row]; result.total = 1;
  await page.route('**/api/v1/analysis/flips?*', (route) => route.fulfill({ json: result }));
  await page.goto('/#/flips');
  const cells = page.getByRole('table', { name: 'Ranked flips', exact: true }).locator('tbody > tr').first().locator('td');
  await expect(cells.nth(1).locator('span')).toHaveText('94,23');
  await expect(cells.nth(2)).toHaveText('12 345,68');
  await expect(cells.nth(3)).toHaveText('12 251,45');
  await expect(cells.nth(4)).toHaveText('8 932,26%');
  await expect(cells.nth(4)).toHaveAttribute('title', row.growth_percent);
  await expect(cells.nth(5)).toHaveText('12 345,68 → 15 000');
  await expect(cells.nth(5)).toHaveAttribute('title', /12345.6789 → 15000.*not the period total/);
  await page.getByRole('button', { name: 'Divine Orb', exact: true }).click();
  await expect(page.getByText('Original native price:', { exact: false })).toContainText('0,0001235 → 94,23');
  const alternatives = page.getByRole('table', { name: 'Divine Orb pair alternatives', exact: true });
  await expect(alternatives).toContainText('12 345,68 → 15 000');
  await expect(alternatives).toContainText('12 251,45 / 8 932,26%');
});

test('arbitrage direct link and reload show warnings, historical disclaimer, native legs and client filtering', async ({ page }) => {
  await service(page);
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/#/arbitrage');
  await expect(page.getByText(/IMPLIED HISTORICAL returns/)).toContainText('nonconcurrent');
  await expect(page.getByLabel('Arbitrage warnings')).toContainText('Missing native market observations');
  await expect(page.getByText(/Actual observation hour:/)).toContainText('Incomplete coverage');
  await page.getByRole('button', { name: 'CHAOS → DIVINE → EXALTED → CHAOS', exact: true }).click();
  await expect(page.getByText('CHAOS → DIVINE: 0.005 · market chaos|divine', { exact: true })).toBeVisible();
  await page.getByLabel('Minimum return %', { exact: true }).fill('6');
  await expect(page.getByRole('table', { name: 'Historical arbitrage loops' })).toContainText('No matching historical loops');
  await page.getByLabel('Minimum return %', { exact: true }).fill('4');
  await page.setViewportSize({ width: 390, height: 800 });
  await expect(page.getByRole('table', { name: 'Historical arbitrage loops' })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.reload();
  await expect(page.getByRole('heading', { name: 'Triangular arbitrage', exact: true })).toBeVisible();
  await page.goto('/#/triangular-arbitrage');
  await expect(page.getByRole('heading', { name: 'Triangular arbitrage', exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test('flip analysis is disabled for unknown league bounds without inventing dates or fetching missing hours', async ({ page }) => {
  const calls = await service(page);
  await page.route('**/api/v1/analysis/period?*', (route) => route.fulfill({ json: {
    league: 'Standard', start: null, end: null, bounds_source: 'unavailable', hours: [], days: [],
    warnings: ['No saved observations or known bounds for this league.'],
  } }));
  await page.goto('/#/flips');
  await expect(page.getByRole('button', { name: 'Analyze flips', exact: true })).toBeDisabled();
  await expect(page.getByLabel('Flip start')).toHaveValue('');
  await expect(page.getByRole('status')).toContainText('League bounds are unavailable');
  expect(calls.some((call) => call.path.endsWith('/analysis/flips') || call.path.endsWith('/ggg/import'))).toBe(false);
});

test('header Refresh retries failed flip period metadata without changing leagues or reloading', async ({ page }) => {
  await service(page);
  let attempts = 0;
  await page.route('**/api/v1/analysis/period?*', (route) => {
    attempts++;
    return attempts === 1 ? route.fulfill({ status: 502, body: 'Temporary restart' })
      : route.fulfill({ json: analysisPeriod() });
  });
  await page.goto('/#/flips');
  await expect(page.getByRole('alert')).toContainText('HTTP 502');
  await expect(page.getByLabel('Flip start')).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Analyze flips', exact: true })).toBeDisabled();
  await page.locator('#refresh').click();
  await expect(page.getByLabel('Flip start')).toBeEnabled();
  await expect(page.getByRole('button', { name: 'Analyze flips', exact: true })).toBeEnabled();
  await expect(page.getByRole('table', { name: 'Ranked flips', exact: true })).toBeVisible();
  await expect(page.getByRole('alert')).toHaveCount(0); expect(attempts).toBe(2);
});

test.describe('mobile period band', () => {
  test.use({ hasTouch: true, viewport: { width: 390, height: 800 } });
  test('commits a captured touch drag only after release', async ({ page }) => {
    const calls = await service(page);
    await page.goto('/#/flips');
    const svg = page.locator('period-band svg');
    await expect(svg).toBeVisible(); await svg.scrollIntoViewIfNeeded();
    const box = (await svg.boundingBox())!;
    const before = calls.filter((call) => call.path.endsWith('/analysis/flips')).length;
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: box.x + 3, y: box.y + box.height / 2 }] });
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: box.x + box.width / 2, y: box.y + box.height / 2 }] });
    expect(calls.filter((call) => call.path.endsWith('/analysis/flips'))).toHaveLength(before);
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    await expect.poll(() => calls.filter((call) => call.path.endsWith('/analysis/flips')).length).toBe(before + 1);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await cdp.detach();
  });
});

for (const timezoneId of ['America/New_York', 'Pacific/Auckland']) {
  test.describe(`flip absolute hours in ${timezoneId}`, () => {
    test.use({ timezoneId });
    test('preserves selected absolute hours through DST and route recreation', async ({ page }) => {
      const calls = await service(page);
      const first = Date.parse('2026-11-01T05:00:00Z') / 1000, last = first + 3600;
      await page.route('**/api/v1/analysis/period?*', (route) => route.fulfill({ json: {
        league: 'Standard', start: first - 3600, end: last + 3600, bounds_source: 'observed',
        hours: [first, last], days: [], warnings: [],
      } }));
      await page.goto('/#/flips');
      await expect(page.getByRole('table', { name: 'Ranked flips', exact: true })).toBeVisible();
      await page.getByRole('button', { name: 'Analyze flips', exact: true }).click();
      await expect.poll(() => Number(calls.filter((call) => call.path.endsWith('/analysis/flips')).at(-1)?.query.get('end'))).toBe(last);
      await page.getByRole('button', { name: 'Triangular arbitrage', exact: true }).click();
      await expect(page.getByRole('heading', { name: 'Triangular arbitrage', exact: true })).toBeVisible();
      await page.getByRole('button', { name: 'Flip analysis', exact: true }).click();
      await expect(page.getByRole('table', { name: 'Ranked flips', exact: true })).toBeVisible();
      const query = calls.filter((call) => call.path.endsWith('/analysis/flips')).at(-1)!.query;
      expect(Number(query.get('start'))).toBe(first); expect(Number(query.get('end'))).toBe(last);
      await expect(page.getByRole('slider', { name: 'Period start', exact: true })).toHaveAttribute('aria-valuenow', String(first));
      await expect(page.getByRole('slider', { name: 'Period end', exact: true })).toHaveAttribute('aria-valuenow', String(last));
    });
  });
}

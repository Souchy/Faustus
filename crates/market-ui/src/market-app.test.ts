import { afterEach, describe, expect, it, vi } from 'vitest';
import { DI, Registration } from 'aurelia';
import { MarketApp } from './market-app';
import { MarketApi } from './core/market-api';
import { gemTypes } from './core/gem-analysis';
import { categorizedGemOverview, gemAnalysis, gemOverview, prices, status } from './testing/fixtures';

afterEach(() => vi.unstubAllGlobals());

function createApp(): MarketApp {
  return DI.createContainer().invoke(MarketApp);
}

function mockService(options: { priceFailure?: boolean; selected?: string | null; categorized?: boolean } = {}) {
  const fetch = vi.fn(async (input: string) => {
    const url = new URL(input, 'http://localhost');
    if (url.pathname.endsWith('/leagues')) return Response.json([{ id: 'Standard', name: 'Standard' }, { id: 'Other', name: 'Other' }]);
    if (url.pathname.endsWith('/status')) return Response.json(status(options.selected === undefined ? 'Standard' : options.selected));
    if (url.pathname.endsWith('/selection')) return Response.json({ id: 'Standard', name: 'Standard' });
    if (url.pathname.endsWith('/markets')) return Response.json([]);
    if (url.pathname.endsWith('/prices')) return options.priceFailure ? new Response('', { status: 502 }) : Response.json(prices());
    if (url.pathname.endsWith('/gems')) return Response.json(options.categorized ? categorizedGemOverview() : gemOverview());
    if (url.pathname.endsWith('/gem-analysis') || url.pathname.endsWith('/gem-analysis/refresh')) {
      const analysis = gemAnalysis(options.categorized ? categorizedGemOverview() : gemOverview(), url.searchParams.get('snapshot') === '1');
      if (options.priceFailure) {
        analysis.data.prices = null;
        analysis.data.price_error = 'Material prices returned HTTP 502';
      }
      return Response.json(analysis);
    }
    throw new Error(`Unexpected request: ${input}`);
  });
  vi.stubGlobal('fetch', fetch);
  return fetch;
}

describe('dashboard state', () => {
  it('explicitly refreshes the latest snapshot without replacing a pinned historical instant', async () => {
    const fetch = mockService();
    const app = createApp();
    await app.loadDashboard();
    expect(fetch.mock.calls.some(([input]) => input.includes('/gem-analysis/refresh'))).toBe(false);
    await app.selectGemSnapshot('1');
    await app.refresh();
    expect(fetch).toHaveBeenCalledWith('/api/v1/gem-analysis/refresh?league=Standard',
      expect.objectContaining({ method: 'POST' }));
    expect(app.selectedGemSnapshot).toBe('1');
    expect(app.gems?.fetched_at).toBe('2026-11-08T12:00:00Z');
    expect(app.latestGemFetchedAt).toBe('2026-11-09T12:00:00Z');
  });

  it('uses editable material prices for the relevant strategies and restores automatic snapshot prices', async () => {
    mockService();
    const app = createApp();
    await app.loadDashboard();
    expect(app.gcpPrice).toBe(1);
    expect(app.vaalPrice).toBe(2);
    app.setStrategy('quality');
    app.setMaterialPrice('gcp', '1.5');
    expect(app.rows[0]).toMatchObject({ materialCost: 30, profit: 30 });
    app.setMaterialPrice('vaal', '3.25');
    app.setStrategy('corrupt');
    expect(app.rows[0]?.profit).toBe(-15.75);
    const row = app.rows[0];
    if (!row) throw new Error('Expected corruption opportunity');
    expect(app.corruptionBreakdown(row)).toContain('12.5% x 140.00c = 17.50c');
    expect(app.corruptionBreakdown(row)).toContain('Expected Net = 87.50c - 103.25c = -15.75c');
    expect(app.corruptionBreakdown(row)).toContain('estimated: lowest listed price in bucket');
    app.setStrategy('level');
    expect(app.rows[0]?.profit).toBe(38);
    await app.selectGemSnapshot('1');
    expect(app.gcpPrice).toBe(1.5);
    expect(app.vaalPrice).toBe(3.25);
    await app.loadDashboard();
    expect(app.gcpPrice).toBe(1.5);
    app.setMaterialPrice('gcp', '');
    app.setMaterialPrice('vaal', '');
    expect(app.gcpPrice).toBe(0.5);
    expect(app.vaalPrice).toBe(0.5);
    expect(app.materialPlaceholder(app.gemPrices, 'gcp')).toBe('0.5');
    app.setMaterialPrice('gcp', '10');
    app.setMaterialPrice('vaal', '10');
    await app.loadDashboard('Other');
    expect(app.gcpPriceText).toBe('');
    expect(app.vaalPriceText).toBe('');
    expect(app.gcpPrice).toBe(1);
    expect(app.prices?.lines[0]?.primary_value).toBe('1');
  });

  it('validates manual prices without falling back and allows zero or explicit prices when the feed fails', async () => {
    mockService({ priceFailure: true });
    const app = createApp();
    await app.loadDashboard();
    expect(app.materialPlaceholder(app.gemPrices, 'gcp')).toBe('Chaos');
    for (const material of ['gcp', 'vaal'] as const) {
      app.setMaterialPrice('gcp', '');
      app.setMaterialPrice('vaal', '');
      app.setStrategy(material === 'gcp' ? 'quality' : 'corrupt');
      app.setMaterialPrice(material, '0');
      expect(app.rows).toHaveLength(1);
      expect(app.rows[0]?.materialCost).toBe(0);
      for (const value of ['-1', 'NaN', 'Infinity', 'invalid']) {
        app.setMaterialPrice(material, value);
        expect(app.rows).toEqual([]);
        expect(app.materialPriceError).toContain('non-negative');
      }
      app.setMaterialPrice(material, '2');
      expect(app.materialPriceError).toBe('');
      expect(app.rows).toHaveLength(1);
      expect(app.gemPriceError).toContain('HTTP 502');
      app.setMaterialPrice(material, '');
      expect(app.rows).toEqual([]);
    }
  });

  it('uses the API instance registered with Aurelia DI', async () => {
    const api = new MarketApi('');
    const leagues = vi.spyOn(api, 'leagues').mockResolvedValue([]);
    vi.spyOn(api, 'status').mockResolvedValue(status());
    const container = DI.createContainer().register(Registration.instance(MarketApi, api));
    const app = container.invoke(MarketApp);
    await app.loadDashboard();
    expect(leagues).toHaveBeenCalledOnce();
    expect(app.error).toContain('no available leagues');
  });

  it('keeps multiple types across strategies, snapshots and refreshes and resets pagination', async () => {
    mockService({ categorized: true });
    const app = createApp();
    await app.loadDashboard();
    expect(app.selectedGemTypes).toBe('all');
    expect(app.gemTypeSummary).toBe('All');
    expect(app.rows).toHaveLength(5);
    app.rowLimit = 100;
    app.setGemTypes(['Exceptional', 'Support']);
    expect(app.rowLimit).toBe(50);
    expect(app.rows.map((row) => row.name)).toEqual(['Empower Support', 'Multistrike Support']);
    expect(app.gemTypeSummary).toBe('2 types');
    app.setStrategy('quality');
    expect(app.selectedGemTypes).toEqual(['Support', 'Exceptional']);
    expect(app.rows.map((row) => row.name)).toEqual(['Empower Support', 'Multistrike Support']);
    await app.selectGemSnapshot('1');
    expect(app.selectedGemTypes).toEqual(['Support', 'Exceptional']);
    expect(app.rows.map((row) => row.name)).toEqual(['Empower Support', 'Multistrike Support']);
    app.setSearch('empower');
    await app.loadDashboard();
    expect(app.selectedGemTypes).toEqual(['Support', 'Exceptional']);
    expect(app.rows.map((row) => row.name)).toEqual(['Empower Support']);
    app.setGemTypes(['Support']);
    expect(app.rows).toHaveLength(0);
    app.setSearch('');
    expect(app.rows.map((row) => row.name)).toEqual(['Multistrike Support']);
    app.sort('gemType');
    expect(app.sortAscending).toBe(true);
  });

  it('toggles types, summarizes selections, and distinguishes an empty selection from All', async () => {
    mockService({ categorized: true });
    const app = createApp();
    await app.loadDashboard();
    app.setGemTypes([]);
    expect(app.gemTypeSummary).toBe('None');
    expect(app.rows).toEqual([]);
    expect(app.gemEmptyState).toContain('Select at least one gem type');
    app.toggleGemType('Transfigured');
    expect(app.gemTypeSummary).toBe('Transfigured');
    app.toggleGemType('Skill');
    expect(app.gemTypeSummary).toBe('2 types');
    expect(app.rows).toHaveLength(2);
    app.toggleGemType('Support');
    expect(app.gemTypeSummary).toBe('3 types');
    app.toggleGemType('Skill');
    expect(app.selectedGemTypes).toEqual(['Support', 'Transfigured']);
    app.selectAllGemTypes();
    expect(app.gemTypeSummary).toBe('All');
    expect(app.selectedGemTypes).toBe('all');
    expect(app.rows).toHaveLength(5);
  });

  it('makes All exclusive with individual types and preserves that mode across snapshots and refreshes', async () => {
    mockService({ categorized: true });
    const app = createApp();
    await app.loadDashboard();
    expect(app.selectedGemTypes).toBe('all');
    expect(app.rows).toHaveLength(5);
    app.toggleGemType('Skill');
    expect(app.selectedGemTypes).toEqual(['Skill']);
    expect(app.rows.map((row) => row.gemType)).toEqual(['Skill']);
    app.toggleGemType('Transfigured');
    expect(app.selectedGemTypes).toEqual(['Skill', 'Transfigured']);
    app.selectAllGemTypes();
    expect(app.selectedGemTypes).toBe('all');
    expect(app.rows).toHaveLength(5);
    await app.selectGemSnapshot('1');
    await app.loadDashboard();
    expect(app.selectedGemTypes).toBe('all');
    app.setGemTypes(gemTypes);
    expect(app.gemTypeSummary).toBe('6 types');
    expect(app.selectedGemTypes).toEqual(gemTypes);
    app.selectAllGemTypes();
    app.selectAllGemTypes();
    expect(app.selectedGemTypes).toBe('all');
  });

  it('chooses a league, selects it on the service and builds opportunities', async () => {
    const fetch = mockService({ selected: null });
    const app = createApp();
    await app.loadDashboard();
    expect(app.error).toBe('');
    expect(app.loading).toBe(false);
    expect(app.selectedLeagueId).toBe('Standard');
    expect(fetch).toHaveBeenCalledWith('/api/v1/selection', expect.objectContaining({ body: '{"id":"Standard"}' }));
    expect(app.rows[0]?.profit).toBe(38);
  });

  it('keeps independently successful data when a provider fails', async () => {
    mockService({ priceFailure: true });
    const app = createApp();
    await app.loadDashboard();
    expect(app.error).toBe('');
    expect(app.priceError).toContain('HTTP 502');
    expect(app.gems?.gems).toHaveLength(gemOverview().gems.length);
    expect(app.rows).toHaveLength(1);
    app.setStrategy('quality');
    expect(app.rows).toHaveLength(0);
    expect(app.gemEmptyState).toContain('unavailable');
  });

  it('retains shared search and uses one Lens with a default price of 50 Chaos', async () => {
    mockService();
    const app = createApp();
    await app.loadDashboard();
    app.setSearch('fire');
    app.rowLimit = 100;
    app.setStrategy('quality');
    expect(app.search).toBe('fire');
    expect(app.rowLimit).toBe(50);
    expect(app.rows[0]?.profit).toBe(40);
    expect(app.lensPriceText).toBe('50');
    app.setStrategy('level');
    app.toggleLens();
    expect(app.rows[0]?.profit).toBe(-12);
    app.setLensPrice('');
    expect(app.gemEmptyState).toContain('positive');
    app.setLensPrice('5');
    expect(app.rows[0]?.profit).toBe(33);
    expect(app.snapshotLabel(new Date(2026, 10, 9, 7, 5).toISOString())).toBe('Nov 9, 07:05');
    expect(app.timestamp('2026-11-09T12:00:00Z')).toBe(
      new Date('2026-11-09T12:00:00Z').toLocaleString('en-CA', {
        year: 'numeric', month: '2-digit', day: '2-digit',
        hour: '2-digit', minute: '2-digit', hourCycle: 'h23', timeZoneName: 'short',
      }),
    );
    expect(app.timestamp(undefined)).toBe('not available');
  });

  it('validates the threshold and updates filtering and badges without including low confidence by default', async () => {
    mockService();
    const app = createApp();
    await app.loadDashboard();
    expect(app.includeLowConfidence).toBe(false);
    expect(app.confidenceThreshold).toBe(10);
    app.setConfidenceThreshold('13');
    expect(app.rows).toHaveLength(0);
    app.toggleConfidence();
    expect(app.rows[0]?.lowConfidence).toBe(true);
    app.setConfidenceThreshold('12');
    expect(app.rows[0]?.lowConfidence).toBe(false);
    for (const input of ['', ' ', '0', '-1', '1.5', 'Infinity', 'NaN', '9007199254740992']) {
      app.setConfidenceThreshold(input);
      expect(app.confidenceError).toContain('whole-number');
      expect(app.rows).toEqual([]);
    }
    app.setConfidenceThreshold('1');
    expect(app.confidenceError).toBe('');
    expect(app.rows).toHaveLength(1);
  });

  it('uses one saved instant for gems, materials and confidence while keeping the prices tab current', async () => {
    mockService();
    const app = createApp();
    await app.loadDashboard();
    expect(app.selectedGemSnapshot).toBe('latest');
    expect(app.latestGemFetchedAt).toBe('2026-11-09T12:00:00Z');
    const latestLabel = app.latestGemLabel;
    expect(app.gemSnapshots.map((snapshot) => snapshot.id)).toEqual([2, 1]);
    app.setSearch('fire');
    app.setStrategy('quality');
    await app.selectGemSnapshot('1');
    expect(app.latestGemLabel).toBe(latestLabel);
    expect(app.rows[0]).toMatchObject({ sourcePrice: 20, targetPrice: 50, materialCost: 10, profit: 20 });
    expect(app.gcpPrice).toBe(0.5);
    expect(app.prices?.lines[0]?.primary_value).toBe('1');
    app.setConfidenceThreshold('11');
    expect(app.rows).toHaveLength(0);
    app.setConfidenceThreshold('5');
    await app.loadDashboard();
    expect(app.selectedGemSnapshot).toBe('1');
    expect(app.search).toBe('fire');
    expect(app.rows[0]?.profit).toBe(20);
    await app.selectGemSnapshot('latest');
    expect(app.rows[0]?.profit).toBe(40);
    await app.selectGemSnapshot('1');
    await app.loadDashboard('Other');
    expect(app.selectedGemSnapshot).toBe('latest');
    expect(app.rows[0]?.profit).toBe(40);
  });

  it('does not replace missing historical material prices with current prices', async () => {
    const fetch = mockService();
    const app = createApp();
    await app.loadDashboard();
    const saved = gemAnalysis(gemOverview(), true);
    saved.data.prices = null;
    saved.data.price_error = 'Material prices returned HTTP 502';
    fetch.mockResolvedValueOnce(Response.json(saved));
    await app.selectGemSnapshot('1');
    expect(app.prices).not.toBeNull();
    expect(app.gcpPrice).toBeNull();
    expect(app.rows[0]?.profit).toBe(19);
    app.setStrategy('quality');
    expect(app.rows).toEqual([]);
    expect(app.gemPriceError).toContain('HTTP 502');
  });

  it('ignores stale snapshot responses after another instant or a league change', async () => {
    const fetch = mockService();
    const app = createApp();
    await app.loadDashboard();
    let resolveOld: ((response: Response) => void) | undefined;
    fetch.mockImplementationOnce(() => new Promise<Response>((resolve) => { resolveOld = resolve; }));
    const old = app.selectGemSnapshot('1');
    await app.selectGemSnapshot('latest');
    resolveOld?.(Response.json(gemAnalysis(gemOverview(), true)));
    await old;
    expect(app.rows[0]?.profit).toBe(38);
    fetch.mockImplementationOnce(() => new Promise<Response>((resolve) => { resolveOld = resolve; }));
    const beforeLeagueChange = app.selectGemSnapshot('1');
    await app.loadDashboard('Other');
    resolveOld?.(Response.json(gemAnalysis(gemOverview(), true)));
    await beforeLeagueChange;
    expect(app.selectedGemSnapshot).toBe('latest');
    expect(app.rows[0]?.profit).toBe(38);
  });

  it('shows snapshot errors without silently switching to Latest and allows recovery', async () => {
    const fetch = mockService();
    const app = createApp();
    await app.loadDashboard();
    fetch.mockResolvedValueOnce(new Response('', { status: 404 }));
    await app.selectGemSnapshot('1');
    expect(app.selectedGemSnapshot).toBe('1');
    expect(app.gemError).toContain('HTTP 404');
    expect(app.rows).toEqual([]);
    expect(app.gemLoading).toBe(false);
    await app.selectGemSnapshot('latest');
    expect(app.gemError).toBe('');
    expect(app.rows[0]?.profit).toBe(38);
  });
  it('does not apply an older request after a newer refresh completes', async () => {
    let resolveOld: ((response: Response) => void) | undefined;
    const fetch = mockService();
    fetch.mockImplementationOnce(() => new Promise<Response>((resolve) => { resolveOld = resolve; }));
    const app = createApp();
    const old = app.loadDashboard();
    await app.loadDashboard();
    resolveOld?.(Response.json([{ id: 'Old', name: 'Old' }]));
    await old;
    expect(app.selectedLeagueId).toBe('Standard');
    expect(app.rows[0]?.profit).toBe(38);
  });

  it('shows actionable connection errors and clears stale data', async () => {
    mockService();
    const app = createApp();
    await app.loadDashboard();
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Failed to fetch')));
    await app.loadDashboard();
    expect(app.error).toContain('external market service');
    expect(app.rows).toEqual([]);
    expect(app.prices).toBeNull();
    expect(app.loading).toBe(false);
  });
});

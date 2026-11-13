import { DI, Registration } from 'aurelia';
import { describe, expect, it, vi } from 'vitest';
import { MarketApi } from '../core/market-api';
import { latestHour, localInput } from '../core/historical';
import { MarketSession } from '../services/market-session';
import { analysisPeriod, arbitrageAnalysis, flipAnalysis, flipSparklines, gggCoverage, gggHistory, gggJob, gggMarket, gggOverview } from '../testing/historical-fixtures';
import { CurrencyExchange } from './currency-exchange/currency-exchange';
import { FlipAnalysis } from './flip-analysis/flip-analysis';
import { TriangularArbitrage } from './triangular-arbitrage/triangular-arbitrage';

function workspace() {
  const api = new MarketApi('');
  vi.spyOn(api, 'analysisPeriod').mockImplementation(async (league) => analysisPeriod(league));
  vi.spyOn(api, 'flipSparklines').mockImplementation(async (_league, start, end, rows) => flipSparklines(rows, start, end));
  const container = DI.createContainer().register(Registration.instance(MarketApi, api), Registration.singleton(MarketSession, MarketSession));
  const session = container.get(MarketSession);
  session.leagues = [{ id: 'standard', name: 'Standard' }];
  session.selectedLeagueId = 'standard';
  return { api, container, session };
}
describe('Historical pages', () => {
  it('uses provider confidence labels, never fabricated confidence or GGG zeroes', () => {
    const { container } = workspace();
    const page = container.invoke(FlipAnalysis);
    const [ggg, ninja] = flipAnalysis().rows;
    expect(page.confidence(ggg!)).toBe('—');
    expect(page.confidenceTitle(ggg!)).toContain('Not applicable');
    expect(page.confidence(ninja!)).toBe('High → Low');
    expect(page.confidence({ ...ninja!, start_confidence: null, end_confidence: undefined })).toBe('Unknown → Unknown');
    expect(page.confidence({ ...ninja!, start_confidence: '', end_confidence: 'Medium' })).toBe('Unknown → Medium');
  });
  it('publishes ranking before its separate displayed-row chart batch resolves', async () => {
    const { api, container } = workspace();
    const result = flipAnalysis();
    vi.spyOn(api, 'flips').mockResolvedValue(result);
    let finish!: (value: ReturnType<typeof flipSparklines>) => void;
    vi.mocked(api.flipSparklines).mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    const page = container.invoke(FlipAnalysis);
    await page.refresh();
    expect(page.result).toBe(result); expect(page.isLoading).toBe(false); expect(page.chartsLoading).toBe(true);
    expect(api.flipSparklines).toHaveBeenCalledWith(result.league, result.start, result.end, result.rows, expect.any(AbortSignal));
    finish(flipSparklines(result.rows)); await vi.waitFor(() => expect(page.chartsLoading).toBe(false));
    expect(page.charts.divine?.paths).toHaveLength(2);
    expect(page.snapshot()).not.toHaveProperty('charts');
    expect(page.snapshot()).not.toHaveProperty('chartRequest');
    page.unbinding();
  });
  it.each(['search', 'filter', 'page', 'league', 'unbind'] as const)('aborts charts on %s and ignores late replies', async (action) => {
    const { api, container, session } = workspace();
    vi.spyOn(api, 'flips').mockImplementation(async (league) => ({ ...flipAnalysis(), league }));
    const finishes: (() => void)[] = [];
    vi.mocked(api.flipSparklines).mockImplementation((_league, start, end, rows) => new Promise((resolve) => {
      finishes.push(() => resolve(flipSparklines(rows, start, end)));
    }));
    const page = container.invoke(FlipAnalysis);
    await page.refresh();
    const signal = vi.mocked(api.flipSparklines).mock.calls[0]![4];
    if (action === 'search') page.searchChangedByUser();
    else if (action === 'filter') { page.category = 'Oils'; await page.refresh(); }
    else if (action === 'page') await page.changePage(1);
    else if (action === 'league') { session.historicalLeague = 'Mirage'; await page.datasetChanged(); }
    else page.unbinding();
    expect(signal.aborted).toBe(true);
    finishes[0]!(); await Promise.resolve();
    expect(page.charts).toEqual({});
    page.unbinding();
    for (const finish of finishes.slice(1)) finish();
    await Promise.resolve();
    expect(page.charts).toEqual({});
  });
  it('surfaces chart errors without removing ranking and rejects omitted or oversized series', async () => {
    const { api, container } = workspace();
    vi.spyOn(api, 'flips').mockResolvedValue(flipAnalysis());
    const page = container.invoke(FlipAnalysis);
    for (const response of [new Error('HTTP 503'), { series: [] }, {
      series: [{ id: 'divine', warnings: [], points: Array.from({ length: 61 }, () => ({ time: 0, price: '1' })) }],
    }]) {
      if (response instanceof Error) vi.mocked(api.flipSparklines).mockRejectedValueOnce(response);
      else vi.mocked(api.flipSparklines).mockResolvedValueOnce(response);
      await page.refresh();
      await vi.waitFor(() => expect(page.chartsLoading).toBe(false));
      expect(page.chartError).toContain('Historical charts unavailable');
      expect(page.rows).toHaveLength(2);
    }
    page.unbinding();
  });
  it('shows selected-pair endpoint item volumes and preserves unknown liquidity for ninja rows', () => {
    const { container } = workspace();
    const page = container.invoke(FlipAnalysis);
    const [exchange, ninja] = flipAnalysis().rows;
    exchange!.alternatives.unshift({ ...exchange!.alternatives[0]!, selected: false, start_volume: '99999', end_volume: '99999' });
    expect(page.volume(exchange!)).toBe('50 → 60');
    expect(page.volumeTitle(exchange!)).toContain('not the period total');
    expect(page.volume(ninja!)).toBe('Unknown');
    expect(page.volume({ ...ninja!, venue: 'exchange', alternatives: exchange!.alternatives })).toBe('Unknown');
    expect(page.volume({ ...exchange!, alternatives: [] })).toBe('Unknown');
  });
  it('formats flip decimals with the shared economy convention without changing API precision', () => {
    const { container } = workspace();
    const page = container.invoke(FlipAnalysis);
    page.result = flipAnalysis();
    page.result.rows[0]!.growth_percent = '8932.258064516129032258064516';
    expect(page.format(page.result.rows[0]!.growth_percent)).toBe('8 932,26');
    expect(page.format('94.23')).toBe('94,23');
    expect(page.format('0.000123456789')).toBe('0,0001235');
    expect(page.result.rows[0]!.growth_percent).toBe('8932.258064516129032258064516');
  });
  it('defaults to flexible starts and preserves strict-start requests and route state', async () => {
    const { api, container } = workspace();
    const fetch = vi.spyOn(api, 'flips').mockResolvedValue(flipAnalysis());
    const page = container.invoke(FlipAnalysis);
    expect(page.strictStart).toBe(false);
    await page.refresh();
    expect(fetch.mock.calls.at(-1)![3]).not.toHaveProperty('strict_start');
    page.strictStart = true; page.offset = 100;
    await page.refresh();
    expect(fetch.mock.calls.at(-1)![3]).toMatchObject({ strict_start: 'true', offset: '0' });
    page.unbinding();
    const restored = container.invoke(FlipAnalysis);
    expect(restored.strictStart).toBe(true);
    restored.strictStart = false;
    await restored.refresh();
    expect(fetch.mock.calls.at(-1)![3]).not.toHaveProperty('strict_start');
  });
  it('sends minimum confidence globally and preserves it across navigation', async () => {
    const { api, container } = workspace();
    const fetch = vi.spyOn(api, 'flips').mockResolvedValue(flipAnalysis());
    const page = container.invoke(FlipAnalysis);
    expect(page.minConfidence).toBe('');
    await page.refresh();
    expect(fetch.mock.calls.at(-1)![3]).not.toHaveProperty('min_confidence');
    page.minConfidence = 'Medium'; page.offset = 100;
    await page.refresh();
    expect(fetch.mock.calls.at(-1)![3]).toMatchObject({ min_confidence: 'Medium', offset: '0' });
    page.unbinding();
    const restored = container.invoke(FlipAnalysis);
    expect(restored.minConfidence).toBe('Medium');
    restored.minConfidence = '';
    await restored.refresh();
    expect(fetch.mock.calls.at(-1)![3]).not.toHaveProperty('min_confidence');
  });
  it('sends exact flip endpoints and server-side filters, preserves returned order and route state', async () => {
    const { api, container } = workspace();
    const fetch = vi.spyOn(api, 'flips').mockResolvedValue(flipAnalysis());
    vi.spyOn(api, 'gggHistory').mockResolvedValue(gggHistory());
    const page = container.invoke(FlipAnalysis);
    page.minPrice = '0'; page.maxPrice = '200'; page.minVolume = '50';
    await page.refresh();
    expect(fetch).toHaveBeenCalledWith('Standard', new Date(page.start).getTime() / 1000, new Date(page.end).getTime() / 1000,
      { min_price: '0', max_price: '200', min_volume: '50', offset: '0', limit: '100',
        sort: 'percent-desc', search: '', category: '', venue: '' }, expect.any(AbortSignal));
    expect(page.rows.map((r) => r.name)).toEqual(['Divine Orb', 'Golden Oil']);
    page.sort = 'value-desc'; expect(page.rows[0]?.name).toBe('Divine Orb');
    page.search = 'Divine'; page.toggle('divine');
    page.unbinding();
    const restored = container.invoke(FlipAnalysis);
    expect(restored.snapshot()).toEqual(page.snapshot());
    expect(restored.isLoading).toBe(false);
    restored.category = 'Oils'; expect(restored.rows).toHaveLength(2);
  });
  it('surfaces invalid min/max without an API request and clears old rows', async () => {
    const { api, container } = workspace();
    const fetch = vi.spyOn(api, 'flips');
    const page = container.invoke(FlipAnalysis);
    page.result = flipAnalysis(); page.minPrice = '20'; page.maxPrice = '10';
    await page.refresh();
    expect(page.error).toContain('must not exceed');
    expect(page.result).toBeNull(); expect(fetch).not.toHaveBeenCalled();
  });
  it('aborts in-flight analysis on unbinding and never applies late results', async () => {
    const { api, container } = workspace();
    let finish!: (result: ReturnType<typeof flipAnalysis>) => void;
    const fetch = vi.spyOn(api, 'flips').mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    const page = container.invoke(FlipAnalysis);
    page.period = analysisPeriod(); page.periodLeague = 'Standard';
    page.start = localInput(page.period.start!); page.end = localInput(page.period.end!);
    const pending = page.refresh(); page.unbinding(); finish(flipAnalysis()); await pending;
    expect(fetch.mock.calls[0]?.[4].aborted).toBe(true);
    expect(page.result).toBeNull();
  });
  it('reads saved GGG hours without starting imports and preserves timeframe/search/history', async () => {
    const { api, container } = workspace();
    vi.spyOn(api, 'gggMarkets').mockResolvedValue(gggOverview());
    vi.spyOn(api, 'gggCoverage').mockResolvedValue(gggCoverage());
    vi.spyOn(api, 'gggImport').mockResolvedValue(null);
    vi.spyOn(api, 'gggHistory').mockResolvedValue(gggHistory());
    const start = vi.spyOn(api, 'startGggImport');
    const page = container.invoke(CurrencyExchange);
    await page.refresh(); await page.selectMarket(gggMarket());
    page.search = 'Divine'; page.unbinding();
    const restored = container.invoke(CurrencyExchange);
    expect(start).not.toHaveBeenCalled();
    expect(restored.snapshot()).toEqual(page.snapshot());
    expect(restored.segments).toHaveLength(2);
  });
  it('can cancel a running import even when draft range controls are invalid', async () => {
    const { api, container } = workspace();
    const cancel = vi.spyOn(api, 'cancelGggImport').mockResolvedValue(gggJob('cancelled'));
    const page = container.invoke(CurrencyExchange);
    page.start = 'invalid'; await page.importHours(true);
    expect(cancel).toHaveBeenCalled(); expect(page.job?.status).toBe('cancelled');
  });
  it('keeps background imports running when unbinding but cancels foreground reads', async () => {
    const { api, container } = workspace();
    const cancel = vi.spyOn(api, 'cancelGggImport');
    vi.spyOn(api, 'startGggImport').mockResolvedValue(gggJob());
    const page = container.invoke(CurrencyExchange);
    await page.importHours();
    page.unbinding();
    expect(cancel).not.toHaveBeenCalled(); expect(page.session.exchangeState?.job?.status).toBe('running');
  });
  it('shows arbitrage request errors, filters returns and restores data-only snapshots', async () => {
    const { api, container } = workspace();
    const fetch = vi.spyOn(api, 'arbitrage').mockRejectedValueOnce(new Error('HTTP 503')).mockResolvedValue(arbitrageAnalysis());
    const page = container.invoke(TriangularArbitrage);
    page.hour = localInput(latestHour()); await page.refresh();
    expect(page.error).toBe('HTTP 503'); expect(page.result).toBeNull();
    await page.refresh(); expect(fetch).toHaveBeenCalledTimes(2);
    page.minReturn = '6'; expect(page.loops).toHaveLength(0);
    page.minReturn = '-1'; expect(page.filterError).toContain('nonnegative');
    page.minReturn = '4'; page.toggle('loop'); page.unbinding();
    expect(container.invoke(TriangularArbitrage).snapshot()).toEqual(page.snapshot());
  });
  it('requests pages of 100 without appending or applying page-local filters', async () => {
    const { api, container } = workspace();
    const page = container.invoke(FlipAnalysis);
    const result = flipAnalysis();
    result.rows = Array.from({ length: 100 }, (_, i) => ({ ...result.rows[0]!, id: `row-${i}`, name: `Item ${i}` }));
    result.total = 250;
    const fetch = vi.spyOn(api, 'flips').mockImplementation(async (_league, _start, _end, filters) => ({ ...result, offset: Number(filters.offset) }));
    await page.refresh();
    expect(page.visibleRows).toHaveLength(100);
    expect(vi.mocked(api.flipSparklines).mock.calls.at(-1)?.[3]).toBe(page.result?.rows);
    expect(vi.mocked(api.flipSparklines).mock.calls.at(-1)?.[3]).toHaveLength(100);
    await page.changePage(1); expect(page.visibleRows).toHaveLength(100);
    expect(vi.mocked(api.flipSparklines).mock.calls.at(-1)?.[3]).toBe(page.result?.rows);
    expect(fetch.mock.calls.at(-1)?.[3].offset).toBe('100');
    page.search = 'Item 249'; page.category = 'Oils'; page.sort = 'value-asc';
    await page.refresh();
    expect(fetch.mock.calls.at(-1)?.[3]).toMatchObject({ offset: '0', search: 'Item 249', category: 'Oils', sort: 'value-asc' });
    expect(page.visibleRows).toHaveLength(100);
    expect(page.categories).toEqual(['Currency', 'Oils']);
  });
  it('loads ninja native supporting history with exact variant keys and no fabricated volumes', async () => {
    const { api, container } = workspace();
    const fetch = vi.spyOn(api, 'economyHistory').mockResolvedValue({ points: [], coverage: { start: null, end: null } });
    const page = container.invoke(FlipAnalysis);
    page.result = flipAnalysis();
    await page.inspect(page.result.rows[1]!);
    expect(fetch).toHaveBeenCalledWith('Standard', 'Oil', 'stash', 'golden-oil', '', expect.any(AbortSignal));
    expect(page.ninjaHistory?.points).toEqual([]);
    expect(page.history).toBeNull();
  });
  it('uses ninja supporting history for Exchange-category rows instead of requesting a GGG pair', async () => {
    const { api, container } = workspace();
    const ninja = vi.spyOn(api, 'economyHistory').mockResolvedValue({ points: [], coverage: { start: null, end: null } });
    const ggg = vi.spyOn(api, 'gggHistory');
    const page = container.invoke(FlipAnalysis); page.result = flipAnalysis();
    const row = { ...page.result.rows[1]!, venue: 'exchange' as const, history_source: 'exchange' };
    await page.inspect(row);
    expect(ninja).toHaveBeenCalled(); expect(ggg).not.toHaveBeenCalled();
  });
  it('resets to actual Mirage dates on league change, preserves same-league custom dates and bounds input requests', async () => {
    const { api, container, session } = workspace();
    const flips = vi.spyOn(api, 'flips').mockResolvedValue(flipAnalysis());
    const page = container.invoke(FlipAnalysis); await page.datasetChanged();
    session.historicalLeague = 'Mirage'; await page.datasetChanged();
    expect(page.start.slice(0, 7)).toBe('2026-03');
    const start = analysisPeriod('Mirage').start! + 3600, end = analysisPeriod('Mirage').end! - 3600;
    page.start = localInput(start); page.end = localInput(end); await page.refresh();
    page.unbinding();
    const restored = container.invoke(FlipAnalysis); await restored.datasetChanged(false);
    expect(restored.start).toBe(localInput(start)); expect(restored.end).toBe(localInput(end));
    restored.start = localInput(start - 10 * 86400); await restored.refresh();
    expect(restored.error).toBe('');
    expect(restored.start).toBe(localInput(analysisPeriod('Mirage').start!));
    expect(restored.end).toBe(localInput(end));
    expect(flips.mock.calls.at(-1)?.[1]).toBe(analysisPeriod('Mirage').start!);
  });
  it('reloads imported bounds on Analyze and clamps each endpoint while retaining filters', async () => {
    const { api, container } = workspace();
    const flips = vi.spyOn(api, 'flips').mockResolvedValue(flipAnalysis());
    const page = container.invoke(FlipAnalysis); await page.datasetChanged();
    const before = analysisPeriod();
    const after = { ...before, start: before.start! + 22 * 3600, end: before.end! - 3600 };
    vi.mocked(api.analysisPeriod).mockResolvedValue(after);
    page.search = 'oil'; page.minVolume = ''; page.offset = 100;
    await page.analyze();
    expect(page.start).toBe(localInput(after.start));
    expect(page.end).toBe(localInput(after.end));
    expect(page.selectedStart).toBe(after.start); expect(page.selectedEnd).toBe(after.end);
    expect(page.search).toBe('oil'); expect(page.offset).toBe(0);
    expect(flips.mock.calls.at(-1)?.slice(1, 3)).toEqual([after.start, after.end]);
    page.start = '2026-02-31T00:00'; await page.analyze();
    expect(page.error).toContain('valid local date');
  });
  it('does not invent endpoints for unavailable periods and ignores stale league metadata', async () => {
    const { api, container, session } = workspace();
    let finish!: (period: ReturnType<typeof analysisPeriod>) => void;
    vi.mocked(api.analysisPeriod).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }))
      .mockResolvedValueOnce({ ...analysisPeriod('Mirage'), start: null, end: null, hours: [], days: [], bounds_source: 'unavailable' });
    const flips = vi.spyOn(api, 'flips');
    const page = container.invoke(FlipAnalysis);
    const old = page.datasetChanged(); session.historicalLeague = 'Mirage';
    await page.datasetChanged(); finish(analysisPeriod()); await old;
    expect(page.periodLeague).toBe('Mirage'); expect(page.canAnalyze).toBe(false);
    expect(page.start).toBe(''); expect(page.end).toBe(''); expect(flips).not.toHaveBeenCalled();
  });
  it('debounces search and resets the server offset', async () => {
    vi.useFakeTimers();
    try {
      const { api, container } = workspace();
      const flips = vi.spyOn(api, 'flips').mockResolvedValue(flipAnalysis());
      const page = container.invoke(FlipAnalysis); await page.datasetChanged(); page.offset = 100;
      page.search = 'Golden'; page.searchChangedByUser();
      await vi.advanceTimersByTimeAsync(200); expect(flips).toHaveBeenCalledTimes(1);
      page.search = 'Golden Oil'; page.searchChangedByUser(); await vi.advanceTimersByTimeAsync(250);
      expect(flips).toHaveBeenCalledTimes(2);
      expect(flips.mock.calls.at(-1)?.[3]).toMatchObject({ search: 'Golden Oil', offset: '0' });
      page.unbinding();
    } finally { vi.useRealTimers(); }
  });
  it('does not restore stale rows if navigation interrupts a debounced server filter', async () => {
    const { api, container } = workspace();
    const flips = vi.spyOn(api, 'flips').mockResolvedValue(flipAnalysis());
    const page = container.invoke(FlipAnalysis); await page.datasetChanged();
    page.search = 'Golden'; page.searchChangedByUser(); page.unbinding();
    const restored = container.invoke(FlipAnalysis);
    expect(restored.result).toBeNull(); await restored.datasetChanged(false);
    expect(flips.mock.calls.at(-1)?.[3].search).toBe('Golden');
  });
  it('retries failed period metadata on page refresh and restores bounded endpoints', async () => {
    const { api, container } = workspace();
    vi.mocked(api.analysisPeriod).mockRejectedValueOnce(new Error('HTTP 502')).mockResolvedValue(analysisPeriod());
    const flips = vi.spyOn(api, 'flips').mockResolvedValue(flipAnalysis());
    const page = container.invoke(FlipAnalysis);
    await page.datasetChanged();
    expect(page.canAnalyze).toBe(false); expect(page.periodLoading).toBe(false);
    expect(page.error).toBe('HTTP 502');
    await page.refresh();
    expect(api.analysisPeriod).toHaveBeenCalledTimes(2);
    expect(page.canAnalyze).toBe(true); expect(page.error).toBe('');
    expect(page.start).toBe(localInput(analysisPeriod().start!));
    expect(page.end).toBe(localInput(analysisPeriod().end!)); expect(flips).toHaveBeenCalledOnce();
  });
  it('stops active import polling on unbinding without cancelling the background job', async () => {
    vi.useFakeTimers();
    const { api, container } = workspace();
    vi.spyOn(api, 'gggMarkets').mockResolvedValue(gggOverview());
    vi.spyOn(api, 'gggCoverage').mockResolvedValue(gggCoverage());
    const poll = vi.spyOn(api, 'gggImport').mockResolvedValue(null);
    vi.spyOn(api, 'startGggImport').mockResolvedValue(gggJob());
    const cancel = vi.spyOn(api, 'cancelGggImport');
    const page = container.invoke(CurrencyExchange);
    try {
      page.attached();
      await vi.advanceTimersByTimeAsync(0);
      await page.importHours();
      const calls = poll.mock.calls.length;
      page.unbinding();
      await vi.advanceTimersByTimeAsync(10_000);
      expect(poll).toHaveBeenCalledTimes(calls);
      expect(cancel).not.toHaveBeenCalled();
    } finally { page.unbinding(); vi.useRealTimers(); }
  });
  it('adds a completed custom GGG import to shared datasets without changing the selected league', async () => {
    vi.useFakeTimers();
    const { api, container, session } = workspace();
    vi.spyOn(api, 'gggMarkets').mockResolvedValue(gggOverview());
    vi.spyOn(api, 'gggCoverage').mockResolvedValue(gggCoverage());
    const poll = vi.spyOn(api, 'gggImport').mockResolvedValue(null);
    vi.spyOn(api, 'startGggImport').mockResolvedValue({ ...gggJob(), league: 'Retained League' });
    const datasets = [{ id: 'Retained League', name: 'Retained League', live: false, archived: false }];
    const catalog = vi.spyOn(api, 'economyLeagues').mockResolvedValue(datasets);
    const page = container.invoke(CurrencyExchange);
    try {
      page.attached();
      await vi.advanceTimersByTimeAsync(0);
      page.customLeague = 'Retained League';
      await page.importHours();
      expect(catalog).not.toHaveBeenCalled();
      poll.mockResolvedValue({ ...gggJob('completed'), league: 'Retained League' });
      await vi.advanceTimersByTimeAsync(1500);
      expect(catalog).toHaveBeenCalledTimes(1);
      expect(session.datasets).toEqual(datasets);
      expect(session.archivedDatasets).toEqual(datasets);
      expect(session.selectedDataset).toBe('standard');
      await session.selectDataset('archive:Retained League');
      expect(session.selectedLeagueName).toBe('Retained League');
    } finally { page.unbinding(); vi.useRealTimers(); }
  });
});

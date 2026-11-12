import { DI, Registration } from 'aurelia';
import { describe, expect, it, vi } from 'vitest';
import { MarketApi } from '../../core/market-api';
import { economyCategories, economyLine, economyOverview, gemAnalysis } from '../../testing/fixtures';
import type { EconomyHistory } from '../../core/api-types';
import { CurrentPrices } from './current-prices';

function workspace() {
  const api = new MarketApi('');
  vi.spyOn(api, 'economyDates').mockResolvedValue(['2026-07-16', '2026-06-25']);
  vi.spyOn(api, 'economy').mockImplementation(async (league, category, source, date) =>
    economyOverview(category, source, league, date));
  const container = DI.createContainer().register(Registration.instance(MarketApi, api));
  const app = container.invoke(CurrentPrices);
  app.categories = economyCategories();
  app.leagues = [{ id: 'Standard', name: 'Standard', live: true }, { id: 'Ancestors', name: 'Ancestors', live: true, archived: true }];
  app.league = 'Standard';
  return { app, api, container };
}

describe('Current prices page', () => {
  it('refreshes the saved gem feed as well as current prices and invalidates cached gem data', async () => {
    const { app, api } = workspace();
    const capture = vi.spyOn(api, 'refreshGemAnalysis').mockResolvedValue(gemAnalysis());
    await app.refresh();
    expect(capture).toHaveBeenCalledWith('Standard', expect.any(AbortSignal));
    expect(app.session.gemRevision).toBe(1);
    expect(api.economy).toHaveBeenCalledWith('Standard', 'Currency', 'exchange', '', expect.any(AbortSignal), true);
    expect(app.overview).not.toBeNull();
    expect(app.isLoading).toBe(false);
  });

  it('shows snapshot-refresh failures while still refreshing current prices', async () => {
    const { app, api } = workspace();
    vi.spyOn(api, 'refreshGemAnalysis').mockRejectedValue(new Error('HTTP 502'));
    await app.refresh();
    expect(app.session.error).toContain('HTTP 502');
    expect(app.session.gemRevision).toBe(0);
    expect(app.overview).not.toBeNull();
    expect(app.isLoading).toBe(false);
  });

  it('does not apply an off-page snapshot refresh after the route unloads', async () => {
    const { app, api } = workspace();
    let finishRefresh: ((analysis: ReturnType<typeof gemAnalysis>) => void) | undefined;
    vi.spyOn(api, 'refreshGemAnalysis').mockImplementationOnce(() => new Promise((resolve) => { finishRefresh = resolve; }));
    const refresh = app.refresh();
    app.unbinding();
    finishRefresh?.(gemAnalysis());
    await refresh;
    expect(api.economy).not.toHaveBeenCalled();
    expect(app.session.gemRevision).toBe(0);
    expect(app.session.error).toBe('');
    expect(app.isLoading).toBe(false);
  });

  it('restores page data into a new route instance without restoring transient view state', () => {
    const { app, container } = workspace();
    app.loading();
    app.overview = economyOverview();
    app.search = 'Divine';
    app.updateLines();
    app.selectedItem = economyLine();
    app.history = {
      points: [15, 16, 17].map((day) => ({
        date: `2026-07-${day}`, price: '1', currency: 'Chaos Orb', granularity: 'daily',
      })),
      coverage: { start: null, end: null },
    };
    app.setPeriodRange(1, 2);
    app.isLoading = app.historyLoading = true;
    app.unbinding();
    expect(app.session.prices).toBeNull();
    const restored = container.invoke(CurrentPrices);
    expect(restored).not.toBe(app);
    expect(restored.search).toBe('Divine');
    expect(restored.selectedItem).toEqual(app.selectedItem);
    expect(restored.history).toEqual(app.history);
    expect([restored.periodStart, restored.periodEnd]).toEqual([1, 2]);
    expect(restored.isLoading || restored.historyLoading).toBe(false);
    expect(restored.hoveredPoint).toBeNull();
  });

  it('clears dataset-specific filters but retains the custom period when a dataset changes off-page', () => {
    const { app, container } = workspace();
    app.search = 'Divine';
    app.selectedItem = economyLine();
    app.history = {
      points: [15, 16, 17].map((day) => ({
        date: `2026-07-${day}`, price: '1', currency: 'Chaos Orb', granularity: 'daily',
      })),
      coverage: { start: null, end: null },
    };
    app.setPeriodRange(1, 2);
    app.unbinding();
    app.session.historicalLeague = 'Ancestors';
    const restored = container.invoke(CurrentPrices);
    expect(restored.search).toBe('');
    expect(restored.selectedItem).toBeNull();
    expect(restored.snapshot().values.customPeriod).toEqual(app.snapshot().values.customPeriod);
  });

  it('ignores a late price response after its route has unloaded', async () => {
    const { app, api } = workspace();
    let finishDates: ((dates: string[]) => void) | undefined;
    vi.mocked(api.economyDates).mockImplementationOnce(() => new Promise((resolve) => { finishDates = resolve; }));
    const loading = app.load();
    app.unbinding();
    finishDates?.(['2026-07-16']);
    await loading;
    expect(api.economy).not.toHaveBeenCalled();
    expect(app.overview).toBeNull();
    expect(app.isLoading).toBe(false);
    expect(app.date).toBe('');
  });

  it('retains custom dates across items, leagues, unavailable history and temporary clamping', async () => {
    const { app, api } = workspace();
    const history = (days: number[], month = '01'): EconomyHistory => ({
      points: days.map((day) => ({
        date: `2026-${month}-${String(day).padStart(2, '0')}`, price: String(day),
        currency: 'Chaos Orb', granularity: 'daily',
      })), coverage: { start: null, end: null },
    });
    const full = history(Array.from({ length: 10 }, (_, index) => index + 1));
    vi.spyOn(api, 'economyHistory').mockResolvedValue(full);
    await app.openItem(economyLine());
    app.setPeriodRange(2, 7);
    const selectedDates = () => [app.historyDates[app.periodStart], app.historyDates[app.periodEnd]];
    expect(selectedDates()).toEqual(['2026-01-03', '2026-01-08']);
    app.closeDetail();
    vi.mocked(api.economyHistory).mockResolvedValue(history([2, 3, 5, 8, 10]));
    await app.openItem(economyLine({ id: 'other' }));
    expect(selectedDates()).toEqual(['2026-01-03', '2026-01-08']);
    expect([app.periodStart, app.periodEnd]).toEqual([1, 3]);
    app.league = 'Other';
    await app.load();
    vi.mocked(api.economyHistory).mockResolvedValue(history([5, 6, 7]));
    await app.openItem(economyLine());
    expect(selectedDates()).toEqual(['2026-01-05', '2026-01-07']);
    vi.mocked(api.economyHistory).mockResolvedValue(history([1, 2], '02'));
    await app.openItem(economyLine({ id: 'non-overlap' }));
    expect(selectedDates()).toEqual(['2026-02-01', '2026-02-01']);
    vi.mocked(api.economyHistory).mockResolvedValue(history([]));
    await app.openItem(economyLine({ id: 'empty' }));
    expect(app.historyRows).toHaveLength(0);
    vi.mocked(api.economyHistory).mockRejectedValueOnce(new Error('HTTP 502'));
    await app.openItem(economyLine({ id: 'unavailable' }));
    expect(app.historyError).toContain('HTTP 502');
    app.league = 'Standard';
    vi.mocked(api.economyHistory).mockResolvedValue(full);
    await app.openItem(economyLine());
    expect(selectedDates()).toEqual(['2026-01-03', '2026-01-08']);
    app.resetPeriod();
    await app.openItem(economyLine({ id: 'reset' }));
    expect(selectedDates()).toEqual(['2026-01-01', '2026-01-10']);
    app.unbinding();
  });

  it('retains trailing-day presets across items instead of replacing them with all history', async () => {
    const { app, api } = workspace();
    vi.spyOn(api, 'economyHistory').mockImplementation(async () => ({
      points: Array.from({ length: 20 }, (_, index) => ({
        date: `2026-01-${String(index + 1).padStart(2, '0')}`, price: '1',
        currency: 'Chaos Orb', granularity: 'daily',
      })), coverage: { start: null, end: null },
    }));
    await app.openItem(economyLine());
    app.setPeriodRange(1, 3);
    app.historyDays = '7';
    app.applyHistoryRange();
    app.closeDetail();
    await app.openItem(economyLine({ id: 'other' }));
    expect(app.historyDays).toBe('7');
    expect(app.historyDates[app.periodStart]).toBe('2026-01-13');
    expect(app.historyDates[app.periodEnd]).toBe('2026-01-20');
    app.unbinding();
  });

  it('orders graph-drag endpoints and clamps selection to the full history', () => {
    const { app } = workspace();
    app.history = { points: Array.from({ length: 10 }, (_, index) => ({
      date: `2026-01-${String(index + 1).padStart(2, '0')}`, price: String(index),
      currency: 'Chaos Orb', granularity: 'daily',
    })), coverage: { start: '2026-01-01', end: '2026-01-10' } };
    app.resetPeriod();
    app.historyDays = '7';
    app.historyLimit = 200;
    app.setHistoryHover(app.series[0]!, 500);
    app.setPeriodRange(8, 2);
    expect([app.periodStart, app.periodEnd]).toEqual([2, 8]);
    expect(app.historyRows).toHaveLength(7);
    expect(app.historyDays).toBe('');
    expect(app.historyLimit).toBe(100);
    expect(app.hoveredPoint).toBeNull();
    app.setPeriodRange(4, -50);
    expect([app.periodStart, app.periodEnd]).toEqual([0, 4]);
    app.setPeriodRange(6, 50);
    expect([app.periodStart, app.periodEnd]).toEqual([6, 9]);
    app.setPeriodRange(4, 4);
    expect(app.periodPoints).toHaveLength(1);
    expect(app.periodIndex(-1)).toBe(0);
    expect(app.periodIndex(2)).toBe(9);
    expect(app.periodIndex(.5)).toBe(4);
    app.history.points = app.history.points.filter((_, index) => [0, 1, 9].includes(index));
    expect(app.periodIndex(.7)).toBe(2);
    expect(app.periodIndex(.1)).toBe(1);
  });

  it('snaps history hover to saved samples and clears it when the period changes', () => {
    const { app } = workspace();
    app.history = { points: [
      { date: '2026-01-01', price: '10', currency: 'Chaos Orb', granularity: 'daily' },
      { date: '2026-01-03', price: '30', currency: 'Chaos Orb', granularity: 'daily' },
      { date: '2026-01-04T12:15:00Z', price: '2', currency: 'Divine Orb', granularity: 'snapshot' },
    ], coverage: { start: '2026-01-01', end: '2026-01-04T12:15:00Z' } };
    app.resetPeriod();
    const chaos = app.series[0]!;
    app.setHistoryHover(chaos, 500);
    expect(app.hoveredPoint?.label).toBe('2026-01-03: 30 Chaos Orb (daily)');
    expect(app.hoveredPoint?.x).toBe(784);
    expect(app.hoveredCurrency).toBe('Chaos Orb');
    app.setHistoryHover(chaos, -100);
    expect(app.hoveredPoint?.label).toContain('2026-01-01: 10');
    app.setHistoryHover(app.series[1]!, 800);
    expect(app.hoveredPoint?.label).toBe('2026-01-04 12:15 UTC: 2 Divine Orb (snapshot)');
    app.clearHistoryHover();
    expect(app.hoveredPoint).toBeNull();
    app.setHistoryHover(chaos, 800);
    app.setPeriodEdge('start', 1);
    expect(app.hoveredPoint).toBeNull();
    expect(app.series[0]!.points).toHaveLength(1);
    app.setHistoryHover(app.series[0]!, 16);
    expect(app.hoveredPoint?.label).toContain('2026-01-03');
    app.resetPeriod();
    expect(app.hoveredPoint).toBeNull();
    app.setHistoryHover({ ...chaos, points: [] }, 10);
    expect(app.hoveredPoint).toBeNull();
  });

  it('selects and clamps a custom graph period and filters the table with it', () => {
    const { app } = workspace();
    app.history = { points: Array.from({ length: 10 }, (_, index) => ({
      date: `2026-01-${String(index + 1).padStart(2, '0')}`, price: String(index + 1),
      currency: 'Chaos Orb', granularity: 'daily',
    })), coverage: { start: '2026-01-01', end: '2026-01-10' } };
    app.resetPeriod();
    expect(app.historyRows).toHaveLength(10);
    app.setPeriodEdge('start', 2);
    app.setPeriodEdge('end', 6);
    expect(app.periodPoints).toHaveLength(5);
    expect(app.historyRows[0]?.date).toBe('2026-01-07');
    app.setPeriodEdge('start', 100);
    expect(app.periodStart).toBe(6);
    app.setPeriodEdge('end', -100);
    expect(app.periodEnd).toBe(6);
    app.historyDays = '7';
    app.applyHistoryRange();
    expect(app.periodStart).toBe(2);
    expect(app.periodEnd).toBe(9);
    app.resetPeriod();
    expect(app.periodStart).toBe(0);
    expect(app.periodPosition(9)).toBe(100);
  });

  it('resolves provider-relative icons against the item CDN and rejects other origins', () => {
    const { app } = workspace();
    expect(app.safeIcon('/gen/image/item.png')).toBe('https://web.poecdn.com/gen/image/item.png');
    expect(app.safeIcon('https://web.poecdn.com/gen/image/item.png')).toBe('https://web.poecdn.com/gen/image/item.png');
    for (const icon of [null, 'http://web.poecdn.com/item.png', 'javascript:alert(1)', '//example.com/item.png',
      'https://user@web.poecdn.com/item.png']) expect(app.safeIcon(icon)).toBeNull();
  });

  it('filters variants and validates listing thresholds without altering the underlying overview', async () => {
    const { app } = workspace();
    app.category = 'SkillGem';
    await app.load();
    expect(app.source).toBe('stash');
    expect(app.rows).toHaveLength(2);
    app.gemLevel = '21';
    app.corrupted = 'yes';
    app.updateLines();
    expect(app.rows[0]?.id).toBe('fire-21c');
    app.minListings = '4';
    app.updateLines();
    expect(app.rows).toHaveLength(0);
    app.minListings = '-1';
    expect(app.filterError).toContain('non-negative');
    app.clearFilters();
    expect(app.rows).toHaveLength(2);
    expect(app.overview?.lines).toHaveLength(2);
    expect(app.facets.find((facet) => facet.field === 'gemLevel')?.options).toEqual([20, 21]);
    app.unbinding();
  });

  it('uses archived stash data without changing live selection or forcing refresh', async () => {
    const { app, api } = workspace();
    app.dataset = 'Ancestors';
    app.date = '2026-11-11';
    await app.load(true);
    expect(app.source).toBe('stash');
    expect(app.date).toBe('2026-07-16');
    expect(app.league).toBe('Standard');
    expect(api.economy).toHaveBeenLastCalledWith('Ancestors', 'Currency', 'stash', '2026-07-16', expect.anything(), false);
    app.unbinding();
  });

  it('ignores stale requests after category changes and surfaces current failures', async () => {
    const { app, api } = workspace();
    let resolveOld: ((value: ReturnType<typeof economyOverview>) => void) | undefined;
    vi.mocked(api.economy).mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve; }));
    const old = app.load();
    await vi.waitFor(() => expect(resolveOld).toBeDefined());
    app.category = 'Map';
    await app.load();
    resolveOld?.(economyOverview());
    await old;
    expect(app.overview?.category).toBe('Map');
    vi.mocked(api.economy).mockRejectedValueOnce(new Error('HTTP 502'));
    await app.load();
    expect(app.overview).toBeNull();
    expect(app.error).toContain('HTTP 502');
    app.unbinding();
  });
});

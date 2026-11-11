import { DI, Registration } from 'aurelia';
import { describe, expect, it, vi } from 'vitest';
import { MarketApi } from '../../core/market-api';
import { economyCategories, economyOverview } from '../../testing/fixtures';
import { EconomyWorkspace } from './economy-workspace';

function workspace() {
  const api = new MarketApi('');
  vi.spyOn(api, 'economyDates').mockResolvedValue(['2026-07-16', '2026-06-25']);
  vi.spyOn(api, 'economy').mockImplementation(async (league, category, source, date) =>
    economyOverview(category, source, league, date));
  const app = DI.createContainer().register(Registration.instance(MarketApi, api)).invoke(EconomyWorkspace);
  app.categories = economyCategories();
  app.leagues = [{ id: 'Standard', name: 'Standard', live: true }, { id: 'Ancestors', name: 'Ancestors', live: true, archived: true }];
  app.league = 'Standard';
  return { app, api };
}

describe('economy workspace state', () => {
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

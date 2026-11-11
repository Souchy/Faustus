import { afterEach, describe, expect, it, vi } from 'vitest';
import { DI, Registration } from 'aurelia';
import { MarketApi } from './market-api';
import { status } from '../testing/fixtures';

afterEach(() => vi.unstubAllGlobals());

describe('direct HTTP transport', () => {
  it('shares one API instance across the application container and its children', () => {
    const container = DI.createContainer().register(Registration.singleton(MarketApi, MarketApi));
    const api = container.get(MarketApi);
    expect(container.get(MarketApi)).toBe(api);
    expect(container.createChild().get(MarketApi)).toBe(api);
  });

  it('uses same-origin requests for web and explicit base URLs for desktop', async () => {
    const fetch = vi.fn(async () => Response.json(status()));
    vi.stubGlobal('fetch', fetch);
    const signal = new AbortController().signal;
    await new MarketApi('').status(signal);
    expect(fetch).toHaveBeenLastCalledWith('/api/v1/status', expect.objectContaining({ signal, credentials: 'omit' }));
    await new MarketApi('http://127.0.0.1:8787/').status(signal);
    expect(fetch).toHaveBeenLastCalledWith('http://127.0.0.1:8787/api/v1/status', expect.anything());
    expect(() => new MarketApi('/relative')).toThrow('absolute HTTP(S)');
  });

  it('encodes league queries and posts JSON selection with no Tauri dependency', async () => {
    const fetch = vi.fn(async () => Response.json({ id: 'A & B', name: 'A & B' }));
    vi.stubGlobal('fetch', fetch);
    const signal = new AbortController().signal;
    const api = new MarketApi('');
    await api.gems('A & B', signal);
    expect(fetch).toHaveBeenLastCalledWith('/api/v1/gems?league=A+%26+B', expect.anything());
    await api.gemAnalysis('A & B', 'latest', signal);
    expect(fetch).toHaveBeenLastCalledWith('/api/v1/gem-analysis?league=A+%26+B', expect.anything());
    await api.gemAnalysis('A & B', '42', signal);
    expect(fetch).toHaveBeenLastCalledWith('/api/v1/gem-analysis?league=A+%26+B&snapshot=42', expect.anything());
    await api.refreshGemAnalysis('A & B', signal);
    expect(fetch).toHaveBeenLastCalledWith('/api/v1/gem-analysis/refresh?league=A+%26+B',
      expect.objectContaining({ method: 'POST', signal, body: '{}' }));
    await api.selectLeague('A & B', signal);
    expect(fetch).toHaveBeenLastCalledWith('/api/v1/selection', expect.objectContaining({
      method: 'POST', body: '{"id":"A & B"}', headers: { Accept: 'application/json', 'Content-Type': 'application/json' },
    }));
  });

  it('encodes economy contexts, exact variants, history and import names', async () => {
    const fetch = vi.fn(async () => Response.json([]));
    vi.stubGlobal('fetch', fetch);
    const api = new MarketApi('');
    const signal = new AbortController().signal;
    await api.economy('A & B', 'SkillGem', 'stash', '2026-07-16', signal);
    expect(fetch).toHaveBeenLastCalledWith('/api/v1/economy/overview?league=A+%26+B&category=SkillGem&source=stash&date=2026-07-16', expect.anything());
    await api.economy('A & B', 'Currency', 'exchange', '', signal, true);
    expect(fetch).toHaveBeenLastCalledWith('/api/v1/economy/refresh?league=A+%26+B&category=Currency&source=exchange',
      expect.objectContaining({ method: 'POST', body: '{}' }));
    await api.economyHistory('Standard', 'SkillGem', 'stash', '21/20 c', '7', signal);
    expect(fetch).toHaveBeenLastCalledWith('/api/v1/economy/history?league=Standard&category=SkillGem&source=stash&item=21%2F20+c&days=7', expect.anything());
    await api.importEconomyArchive('Phrecia 2.0', signal);
    expect(fetch).toHaveBeenLastCalledWith('/api/v1/economy/import',
      expect.objectContaining({ method: 'POST', body: '{"name":"Phrecia 2.0"}' }));
  });

  it('surfaces HTTP, network and JSON errors', async () => {
    const fetch = vi.fn().mockResolvedValue(new Response('', { status: 502 }));
    vi.stubGlobal('fetch', fetch);
    const api = new MarketApi('');
    const signal = new AbortController().signal;
    await expect(api.gems('Standard', signal)).rejects.toThrow('HTTP 502');
    fetch.mockRejectedValueOnce(new TypeError('Failed to fetch'));
    await expect(api.status(signal)).rejects.toThrow('Failed to fetch');
    fetch.mockResolvedValueOnce(new Response('not json'));
    await expect(api.status(signal)).rejects.toThrow();
  });
});

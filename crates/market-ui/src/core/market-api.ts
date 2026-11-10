import type {
  CurrentExchangePriceOverview, ExchangeMarketSnapshot, GemAnalysisOverview, GemMarketOverview,
  LeagueOption, ServiceStatus,
} from './api-types';

export class MarketApi {
  private readonly baseUrl: string;

  constructor(baseUrl = import.meta.env.VITE_API_BASE_URL ?? '') {
    this.baseUrl = baseUrl.replace(/\/+$/, '');
    if (this.baseUrl && !/^https?:\/\//i.test(this.baseUrl)) {
      throw new Error('VITE_API_BASE_URL must be an absolute HTTP(S) URL or empty for same-origin requests.');
    }
  }

  private async request<T>(path: string, signal: AbortSignal, body?: unknown): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: body === undefined ? 'GET' : 'POST',
      headers: body === undefined ? { Accept: 'application/json' }
        : { Accept: 'application/json', 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal,
      credentials: 'omit',
    });
    if (!response.ok) {
      throw new Error(`${path.split('?')[0]} returned HTTP ${response.status}`);
    }
    return response.json() as Promise<T>;
  }

  leagues(signal: AbortSignal): Promise<LeagueOption[]> {
    return this.request('/api/v1/leagues', signal);
  }

  status(signal: AbortSignal): Promise<ServiceStatus> {
    return this.request('/api/v1/status', signal);
  }

  selectLeague(id: string, signal: AbortSignal): Promise<LeagueOption> {
    return this.request('/api/v1/selection', signal, { id });
  }

  markets(league: string, signal: AbortSignal): Promise<ExchangeMarketSnapshot[]> {
    return this.request(`/api/v1/exchange/markets?${new URLSearchParams({ league, limit: '100' })}`, signal);
  }

  prices(league: string, signal: AbortSignal): Promise<CurrentExchangePriceOverview> {
    return this.request(`/api/v1/exchange/prices?${new URLSearchParams({ league, type: 'Currency' })}`, signal);
  }

  gems(league: string, signal: AbortSignal): Promise<GemMarketOverview> {
    return this.request(`/api/v1/gems?${new URLSearchParams({ league })}`, signal);
  }

  gemAnalysis(league: string, snapshot: string, signal: AbortSignal): Promise<GemAnalysisOverview> {
    const query = new URLSearchParams({ league });
    if (snapshot !== 'latest') query.set('snapshot', snapshot);
    return this.request(`/api/v1/gem-analysis?${query}`, signal);
  }
}

import type {
  CurrentExchangePriceOverview, ExchangeMarketSnapshot, GemAnalysisOverview, GemMarketOverview,
  LeagueOption, ServiceStatus, EconomyCategory, EconomyLeague, EconomyOverview, EconomySource,
  EconomyHistory, EconomyArchive, GggOverview, GggCoverage, GggHistory, GggImportJob, FlipAnalysis, FlipRow, FlipSparklines, ArbitrageAnalysis, AnalysisPeriod,
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
      const detail = (await response.text()).slice(0, 500);
      throw new Error(`${path.split('?')[0]} returned HTTP ${response.status}${detail ? `: ${detail}` : ''}`);
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

  refreshGemAnalysis(league: string, signal: AbortSignal): Promise<GemAnalysisOverview> {
    return this.request(`/api/v1/gem-analysis/refresh?${new URLSearchParams({ league })}`, signal, {});
  }

  economyCategories(signal: AbortSignal): Promise<EconomyCategory[]> {
    return this.request('/api/v1/economy/categories', signal);
  }

  economyLeagues(signal: AbortSignal): Promise<EconomyLeague[]> {
    return this.request('/api/v1/economy/leagues', signal);
  }

  economy(league: string, category: string, source: EconomySource, date: string, signal: AbortSignal,
    refresh = false): Promise<EconomyOverview> {
    const query = new URLSearchParams({ league, category, source });
    if (date) query.set('date', date);
    return this.request(`/api/v1/economy/${refresh ? 'refresh' : 'overview'}?${query}`, signal, refresh ? {} : undefined);
  }

  economyDates(league: string, category: string, source: EconomySource, signal: AbortSignal): Promise<string[]> {
    return this.request(`/api/v1/economy/dates?${new URLSearchParams({ league, category, source })}`, signal);
  }

  economyHistory(league: string, category: string, source: EconomySource, item: string, days: string,
    signal: AbortSignal): Promise<EconomyHistory> {
    const query = new URLSearchParams({ league, category, source, item });
    if (days) query.set('days', days);
    return this.request(`/api/v1/economy/history?${query}`, signal);
  }

  economyArchives(signal: AbortSignal, refresh = false): Promise<EconomyArchive[]> {
    return this.request(`/api/v1/economy/archives${refresh ? '?refresh=true' : ''}`, signal);
  }

  importEconomyArchive(name: string, signal: AbortSignal): Promise<EconomyArchive> {
    return this.request('/api/v1/economy/import', signal, { name });
  }

  gggMarkets(league: string, hour: number, signal: AbortSignal): Promise<GggOverview> {
    return this.request(`/api/v1/ggg/markets?${new URLSearchParams({ league, hour: String(hour) })}`, signal);
  }
  gggCoverage(league: string, start: number, end: number, signal: AbortSignal): Promise<GggCoverage> {
    return this.request(`/api/v1/ggg/coverage?${new URLSearchParams({ league, start: String(start), end: String(end) })}`, signal);
  }
  gggHistory(league: string, market: string, start: number, end: number, signal: AbortSignal): Promise<GggHistory> {
    return this.request(`/api/v1/ggg/history?${new URLSearchParams({ league, market, start: String(start), end: String(end) })}`, signal);
  }
  gggImport(league: string, signal: AbortSignal): Promise<GggImportJob | null> {
    return this.request(`/api/v1/ggg/import?${new URLSearchParams({ league })}`, signal);
  }
  startGggImport(league: string, start: number, end: number, signal: AbortSignal): Promise<GggImportJob> {
    return this.request('/api/v1/ggg/import', signal, { league, start, end });
  }
  cancelGggImport(league: string, signal: AbortSignal): Promise<GggImportJob | null> {
    return this.request('/api/v1/ggg/cancel', signal, { league });
  }
  analysisPeriod(league: string, signal: AbortSignal): Promise<AnalysisPeriod> {
    return this.request(`/api/v1/analysis/period?${new URLSearchParams({ league })}`, signal);
  }
  flips(league: string, start: number, end: number, filters: Record<string, string>, signal: AbortSignal): Promise<FlipAnalysis> {
    return this.request(`/api/v1/analysis/flips?${new URLSearchParams({ league, start: String(start), end: String(end), ...filters })}`, signal);
  }
  async flipSparklines(league: string, start: number, end: number, rows: FlipRow[], signal: AbortSignal): Promise<FlipSparklines> {
    if (rows.length > 100) throw new Error('Sparkline batches support at most 100 displayed rows.');
    return this.request('/api/v1/analysis/sparklines', signal, { league, start, end, rows });
  }
  arbitrage(league: string, hour: number, minVolume: string | undefined, signal: AbortSignal): Promise<ArbitrageAnalysis> {
    const query = new URLSearchParams({ league, hour: String(hour) });
    if (minVolume !== undefined) query.set('min_volume', minVolume);
    return this.request(`/api/v1/analysis/arbitrage?${query}`, signal);
  }
}

import { resolve } from 'aurelia';
import type { GggCoverage, GggHistory, GggImportJob, GggMarket, GggOverview } from '../../core/api-types';
import { MarketApi } from '../../core/market-api';
import { HOUR, latestHour, localInput, localDate, meanRatio, parseHour, parseRange, priceSegments } from '../../core/historical';
import { MarketSession } from '../../services/market-session';

export class CurrencyExchange {
  private readonly api = resolve(MarketApi);
  readonly session = resolve(MarketSession);
  private request: AbortController | null = null;
  private detailRequest: AbortController | null = null;
  private pollRequest: AbortController | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private active = false;
  private loadedLeague = '';
  private loadedDataset = '';
  private pollingLeague = '';
  hour = localInput(latestHour());
  start = localInput(latestHour() - 23 * HOUR);
  end = localInput(latestHour());
  customLeague = '';
  search = '';
  limit = 100;
  historyLimit = 100;
  overview: GggOverview | null = null;
  coverage: GggCoverage | null = null;
  job: GggImportJob | null = null;
  selected: GggMarket | null = null;
  history: GggHistory | null = null;
  error = '';
  historyError = '';
  isLoading = false;
  historyLoading = false;
  date = localDate;
  ratio(market: GggMarket): string { return meanRatio(market)?.toLocaleString('en-CA', { maximumSignificantDigits: 8 }) ?? '—'; }
  get league(): string { return this.customLeague.trim() || this.session.selectedLeagueName; }
  get markets(): GggMarket[] {
    const search = this.search.toLocaleLowerCase();
    return this.overview?.markets.filter((m) => `${m.a.name} ${m.b.name} ${m.id}`.toLocaleLowerCase().includes(search)) ?? [];
  }
  get segments(): string[] { return priceSegments(this.history); }
  get visibleMarkets(): GggMarket[] { return this.markets.slice(0, this.limit); }
  constructor() {
    const saved = this.session.exchangeState;
    if (saved) {
      Object.assign(this, saved);
      if (this.loadedDataset !== this.session.selectedDataset) {
        this.customLeague = ''; this.overview = null; this.coverage = null; this.job = null;
        this.selected = null; this.history = null;
      }
    }
  }
  loading(): void { this.session.page = 'exchange'; this.session.exchange = this; }
  attached(): void {
    this.active = true;
    void this.initialize();
  }
  private async initialize(): Promise<void> {
    await this.session.ensureLoaded();
    if (!this.active) return;
    if (!this.overview || this.loadedLeague !== this.league) await this.refresh();
    else {
      void this.poll();
      if (this.selected && !this.history) void this.selectMarket(this.selected);
    }
  }
  snapshot() {
    return { hour: this.hour, start: this.start, end: this.end, search: this.search, customLeague: this.customLeague,
      overview: this.overview, coverage: this.coverage, job: this.job, selected: this.selected,
      history: this.history, loadedLeague: this.loadedLeague, loadedDataset: this.loadedDataset, pollingLeague: this.pollingLeague,
      limit: this.limit, historyLimit: this.historyLimit };
  }
  unbinding(): void {
    this.active = false;
    this.request?.abort(); this.detailRequest?.abort(); this.stopPoll();
    this.session.exchangeState = this.snapshot();
    if (this.session.exchange === this) this.session.exchange = null;
  }
  async datasetChanged(): Promise<void> {
    this.customLeague = '';
    this.selected = null; this.history = null;
    await this.refresh();
  }
  async refresh(): Promise<void> {
    this.request?.abort(); this.detailRequest?.abort(); this.stopPoll();
    const request = this.request = new AbortController();
    this.isLoading = true; this.error = ''; this.historyError = '';
    this.overview = null; this.coverage = null; this.history = null; this.job = null;
    this.limit = 100;
    try {
      const hour = parseHour(this.hour), [start, end] = parseRange(this.start, this.end);
      const league = this.league;
      const [overview, coverage, job] = await Promise.all([
        this.api.gggMarkets(league, hour, request.signal),
        this.api.gggCoverage(league, start, end, request.signal),
        this.api.gggImport(league, request.signal),
      ]);
      if (request.signal.aborted) return;
      this.overview = overview; this.coverage = coverage; this.job = job; this.loadedLeague = league;
      this.loadedDataset = this.session.selectedDataset;
      this.pollingLeague = league;
      await this.refreshCompletedDatasets(job, request.signal);
      if (request.signal.aborted) return;
      if (this.selected) {
        this.selected = overview.markets.find((m) => m.id === this.selected?.id) ?? this.selected;
        void this.selectMarket(this.selected);
      }
      this.schedulePoll();
    } catch (error) {
      if (!request.signal.aborted) this.error = String(error instanceof Error ? error.message : error);
    } finally { if (this.request === request) this.isLoading = false; }
  }
  async importHours(cancel = false): Promise<void> {
    this.request?.abort(); this.stopPoll();
    const request = this.request = new AbortController();
    this.isLoading = true; this.error = '';
    try {
      const league = this.league;
      let job: GggImportJob | null;
      if (cancel) job = await this.api.cancelGggImport(this.pollingLeague || league, request.signal);
      else {
        const [start, end] = parseRange(this.start, this.end);
        job = await this.api.startGggImport(league, start, end, request.signal);
        if (!request.signal.aborted) this.pollingLeague = league;
      }
      if (request.signal.aborted) return;
      this.job = job;
      await this.refreshCompletedDatasets(job, request.signal);
      if (!request.signal.aborted) this.schedulePoll();
    } catch (error) {
      if (!request.signal.aborted) this.error = error instanceof Error ? error.message : String(error);
    } finally { if (this.request === request) this.isLoading = false; }
  }
  private stopPoll(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null; this.pollRequest?.abort();
  }
  private schedulePoll(): void {
    if (this.active && this.job?.status === 'running') this.timer = setTimeout(() => { void this.poll(); }, 1500);
  }
  private async refreshCompletedDatasets(job: GggImportJob | null, signal: AbortSignal): Promise<void> {
    if (job?.status !== 'completed') return;
    const datasets = await this.api.economyLeagues(signal);
    if (!signal.aborted) this.session.datasets = datasets;
  }
  private async poll(): Promise<void> {
    this.stopPoll();
    const request = this.pollRequest = new AbortController();
    try {
      const league = this.pollingLeague || this.league;
      const job = await this.api.gggImport(league, request.signal);
      if (request.signal.aborted || !this.active) return;
      this.job = job;
      const [start, end]: [number, number] = this.coverage && this.coverage.league === league
        ? [this.coverage.start, this.coverage.end] : job ? [job.start, job.end] : parseRange(this.start, this.end);
      const coverage = await this.api.gggCoverage(league, start, end, request.signal);
      if (request.signal.aborted) return;
      this.coverage = coverage;
      if (job?.status === 'completed' && league === this.league) await this.refresh();
      else {
        await this.refreshCompletedDatasets(job, request.signal);
        if (!request.signal.aborted) this.schedulePoll();
      }
    } catch (error) {
      if (!request.signal.aborted) this.error = `Import status: ${error instanceof Error ? error.message : String(error)}`;
    }
  }
  async selectMarket(market: GggMarket): Promise<void> {
    this.detailRequest?.abort();
    const request = this.detailRequest = new AbortController();
    this.selected = market; this.history = null; this.historyError = ''; this.historyLoading = true; this.historyLimit = 100;
    try {
      const [start, end] = parseRange(this.start, this.end);
      const history = await this.api.gggHistory(this.league, market.id, start, end, request.signal);
      if (!request.signal.aborted) this.history = history;
    } catch (error) {
      if (!request.signal.aborted) this.historyError = error instanceof Error ? error.message : String(error);
    } finally { if (this.detailRequest === request) this.historyLoading = false; }
  }
}

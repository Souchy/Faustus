import { resolve } from 'aurelia';
import type { ArbitrageAnalysis, ArbitrageLoop } from '../../core/api-types';
import { MarketApi } from '../../core/market-api';
import { decimal, latestHour, localInput, localDate, parseHour } from '../../core/historical';
import { MarketSession } from '../../services/market-session';

export class TriangularArbitrage {
  private readonly api = resolve(MarketApi);
  readonly session = resolve(MarketSession);
  private request: AbortController | null = null;
  private active = false;
  private loadedLeague = '';
  hour = localInput(latestHour());
  minVolume = '';
  minReturn = '';
  expanded: string[] = [];
  limit = 100;
  result: ArbitrageAnalysis | null = null;
  error = '';
  isLoading = false;
  date = localDate;
  get filterError(): string {
    try { decimal(this.minReturn, 'Minimum return'); return ''; } catch (error) { return (error as Error).message; }
  }
  get loops(): ArbitrageLoop[] {
    if (this.filterError) return [];
    return (this.result?.loops ?? []).filter((loop) => !this.minReturn || Number(loop.return_percent) >= Number(this.minReturn));
  }
  get visibleLoops(): ArbitrageLoop[] { return this.loops.slice(0, this.limit); }
  constructor() { if (this.session.triangularArbitrageState) Object.assign(this, this.session.triangularArbitrageState); }
  loading(): void { this.session.page = 'triangular-arbitrage'; this.session.triangularArbitrage = this; }
  attached(): void { this.active = true; void this.initialize(); }
  private async initialize(): Promise<void> {
    await this.session.ensureLoaded();
    if (this.active && (!this.result || this.loadedLeague !== this.session.selectedLeagueName)) await this.refresh();
  }
  snapshot() {
    return { hour: this.hour, minVolume: this.minVolume, minReturn: this.minReturn,
      expanded: this.expanded, result: this.result, loadedLeague: this.loadedLeague, limit: this.limit };
  }
  unbinding(): void {
    this.active = false; this.request?.abort();
    this.session.triangularArbitrageState = this.snapshot();
    if (this.session.triangularArbitrage === this) this.session.triangularArbitrage = null;
  }
  toggle(id: string): void { this.expanded = this.expanded.includes(id) ? this.expanded.filter((x) => x !== id) : [...this.expanded, id]; }
  async refresh(): Promise<void> {
    this.request?.abort();
    const request = this.request = new AbortController();
    this.isLoading = true; this.error = ''; this.result = null;
    this.limit = 100;
    try {
      const hour = parseHour(this.hour), volume = decimal(this.minVolume, 'Minimum exchange item volume');
      const league = this.session.selectedLeagueName;
      const result = await this.api.arbitrage(league, hour, volume, request.signal);
      if (!request.signal.aborted) { this.result = result; this.loadedLeague = league; }
    } catch (error) {
      if (!request.signal.aborted) this.error = error instanceof Error ? error.message : String(error);
    } finally { if (this.request === request) this.isLoading = false; }
  }
}

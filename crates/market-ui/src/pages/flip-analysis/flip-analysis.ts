import { resolve } from 'aurelia';
import type { AnalysisPeriod, EconomyHistory, FlipAnalysis as FlipAnalysisResult, FlipRow, GggHistory } from '../../core/api-types';
import { MarketApi } from '../../core/market-api';
import { formatNumber } from '../../core/economy';
import { flipSparkline, type FlipSparklineView } from '../../core/flip-sparkline';
import { decimal, latestHour, localInput, localDate, nativeRatioLabel, priceSegments } from '../../core/historical';
import { observationPoints } from '../../components/period-band/period-band';
import { MarketSession } from '../../services/market-session';

export class FlipAnalysis {
  private readonly api = resolve(MarketApi);
  readonly session = resolve(MarketSession);
  private request: AbortController | null = null;
  private detailRequest: AbortController | null = null;
  private periodRequest: AbortController | null = null;
  private chartRequest: AbortController | null = null;
  private searchTimer: ReturnType<typeof setTimeout> | null = null;
  private active = false;
  private loadedLeague = '';
  start = '';
  end = '';
  selectedStart = 0;
  selectedEnd = 0;
  startPicker?: HTMLInputElement;
  endPicker?: HTMLInputElement;
  period: AnalysisPeriod | null = null;
  periodLeague = '';
  periodLoading = false;
  minPrice = '';
  maxPrice = '';
  minVolume = '';
  minConfidence = '';
  strictStart = false;
  search = '';
  category = '';
  venue = '';
  sort = 'percent-desc';
  expanded: string[] = [];
  offset = 0;
  excludedLimit = 100;
  historyRow = '';
  history: GggHistory | null = null;
  ninjaHistory: EconomyHistory | null = null;
  historyLimit = 100;
  historyError = '';
  historyLoading = false;
  result: FlipAnalysisResult | null = null;
  error = '';
  isLoading = false;
  charts: Record<string, FlipSparklineView> = {};
  chartsLoading = false;
  chartError = '';
  date = localDate;
  format = formatNumber;
  nativeRatioLabel = nativeRatioLabel;
  get segments(): string[] { return priceSegments(this.history); }
  get visibleRows(): FlipRow[] { return this.rows; }
  get rows(): FlipRow[] { return this.result?.rows ?? []; }
  get categories(): string[] { return this.result?.categories ?? []; }
  confidence(row: FlipRow): string {
    if (row.source === 'GGG Exchange') return '—';
    return `${row.start_confidence?.trim() || 'Unknown'} → ${row.end_confidence?.trim() || 'Unknown'}`;
  }
  confidenceTitle(row: FlipRow): string {
    return row.source === 'GGG Exchange'
      ? 'Not applicable: GGG reports actual traded volume, not poe.ninja provider confidence.'
      : 'poe.ninja provider confidence at the start → end observations. Missing labels are Unknown.';
  }
  cancelSparklines(): void {
    this.chartRequest?.abort(); this.chartRequest = null;
    this.charts = {}; this.chartsLoading = false; this.chartError = '';
  }
  private async loadSparklines(result: FlipAnalysisResult): Promise<void> {
    this.cancelSparklines();
    if (!result.rows.length) return;
    const request = this.chartRequest = new AbortController();
    this.chartsLoading = true;
    try {
      const response = await this.api.flipSparklines(result.league, result.start, result.end, result.rows, request.signal);
      if (request.signal.aborted || this.result !== result || result.league !== this.session.selectedLeagueName) return;
      const charts: Record<string, FlipSparklineView> = {};
      for (const row of result.rows) {
        const series = response.series.find((series) => series.id === row.id);
        if (!series) throw new Error(`Missing chart series for ${row.name}.`);
        if (series.points.length > 60) throw new Error('Chart series exceeds the 60-point limit.');
        charts[row.id] = flipSparkline(series, result.start, result.end);
      }
      this.charts = charts;
    } catch (error) {
      if (!request.signal.aborted && this.result === result) {
        this.chartError = `Historical charts unavailable: ${error instanceof Error ? error.message : String(error)}`;
      }
    } finally {
      if (this.chartRequest === request) { this.chartsLoading = false; this.chartRequest = null; }
    }
  }
  volume(row: FlipRow): string {
    const pair = row.source === 'GGG Exchange' ? row.alternatives.find((pair) => pair.selected) : undefined;
    return pair ? `${this.format(pair.start_volume)} → ${this.format(pair.end_volume)}` : 'Unknown';
  }
  volumeTitle(row: FlipRow): string {
    const pair = row.source === 'GGG Exchange' ? row.alternatives.find((pair) => pair.selected) : undefined;
    return pair
      ? `${pair.start_volume} → ${pair.end_volume} item units traded in the start/end hours for the selected ${pair.quote} pair; not the period total.`
      : 'Comparable item-unit traded volume is unavailable for this source.';
  }
  get minimum(): number { return this.period?.start ?? 0; }
  get maximum(): number { return Math.min(this.period?.end ?? 0, latestHour()); }
  get minInput(): string { return this.period?.start == null ? '' : localInput(this.minimum); }
  get maxInput(): string { return this.period?.end == null ? '' : localInput(this.maximum); }
  get startText(): string { return this.start.replace('T', ' '); }
  set startText(value: string) { this.start = value.replace(' ', 'T'); }
  get endText(): string { return this.end.replace('T', ' '); }
  set endText(value: string) { this.end = value.replace(' ', 'T'); }
  get canAnalyze(): boolean {
    return this.periodLeague === this.session.selectedLeagueName && this.period?.start != null
      && this.period?.end != null && this.maximum > this.minimum;
  }
  get availabilityMessage(): string {
    if (this.periodLoading) return 'Loading saved period metadata…';
    if (!this.period) return 'Saved period metadata is unavailable. Analysis is disabled.';
    if (!this.canAnalyze) return 'League bounds are unavailable or contain fewer than two completed hours. Analysis is disabled.';
    const points = observationPoints(this.period.hours, this.period.days);
    if (!this.start || !this.end) return 'Fewer than two recorded endpoints are available. Choose endpoints within league bounds; missing data is not interpolated.';
    if (!points.includes(this.selectedStart) || !points.includes(this.selectedEnd)) return 'One or both endpoints have no recorded observation. Missing item endpoints are excluded, never interpolated.';
    return 'Lines mark actual recorded hours / UTC dataset days; item coverage can differ.';
  }
  closeInformation(event: KeyboardEvent): void {
    if (event.key === 'Escape' && event.currentTarget instanceof HTMLDetailsElement) {
      event.preventDefault();
      event.currentTarget.open = false;
      event.currentTarget.querySelector('summary')?.focus();
    }
  }
  timestamp(value: string): string {
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? value : localDate(date.getTime() / 1000);
  }
  constructor() { if (this.session.flipState) Object.assign(this, this.session.flipState); }
  loading(): void { this.session.page = 'flips'; this.session.flips = this; }
  attached(): void { this.active = true; void this.initialize(); }
  private async initialize(): Promise<void> {
    await this.session.ensureLoaded();
    if (this.active) await this.datasetChanged(false);
  }
  snapshot() {
    return { start: this.start, end: this.end, minPrice: this.minPrice, maxPrice: this.maxPrice,
      minVolume: this.minVolume, minConfidence: this.minConfidence, strictStart: this.strictStart, search: this.search, category: this.category, venue: this.venue,
      sort: this.sort, expanded: this.expanded, result: this.result, loadedLeague: this.loadedLeague,
      offset: this.offset, period: this.period, periodLeague: this.periodLeague, selectedStart: this.selectedStart,
      selectedEnd: this.selectedEnd, excludedLimit: this.excludedLimit, historyRow: this.historyRow, history: this.history,
      ninjaHistory: this.ninjaHistory, historyLimit: this.historyLimit };
  }
  unbinding(): void {
    this.cancelSparklines();
    this.active = false; this.request?.abort(); this.detailRequest?.abort();
    this.periodRequest?.abort();
    if (this.searchTimer || this.isLoading) this.result = null;
    if (this.searchTimer) { clearTimeout(this.searchTimer); this.searchTimer = null; }
    this.session.flipState = this.snapshot();
    if (this.session.flips === this) this.session.flips = null;
  }
  toggle(id: string): void {
    const opening = !this.expanded.includes(id);
    this.expanded = opening ? [...this.expanded, id] : this.expanded.filter((x) => x !== id);
    const row = this.result?.rows.find((item) => item.id === id);
    if (opening && row && (row.source === 'GGG Exchange' || row.history_item)) void this.inspect(row);
  }
  async inspect(row: FlipRow): Promise<void> {
    this.detailRequest?.abort();
    const request = this.detailRequest = new AbortController();
    this.historyRow = row.id; this.history = null; this.ninjaHistory = null; this.historyError = ''; this.historyLoading = true;
    this.historyLimit = 100;
    try {
      if (!this.result) return;
      if (row.source === 'GGG Exchange' && row.alternatives.some((pair) => pair.selected)) {
        const market = row.alternatives.find((pair) => pair.selected)!.market;
        const history = await this.api.gggHistory(this.result.league, market, this.result.start, this.result.end, request.signal);
        if (!request.signal.aborted) this.history = history;
      } else if (row.history_category && row.history_item && (row.history_source === 'exchange' || row.history_source === 'stash')) {
        const history = await this.api.economyHistory(this.result.league, row.history_category, row.history_source,
          row.history_item, '', request.signal);
        if (!request.signal.aborted) this.ninjaHistory = history;
      } else throw new Error('Supporting history is unavailable for this source.');
    } catch (error) {
      if (!request.signal.aborted) this.historyError = error instanceof Error ? error.message : String(error);
    } finally { if (this.detailRequest === request) this.historyLoading = false; }
  }
  async datasetChanged(force = true): Promise<void> {
    this.cancelSparklines();
    this.periodRequest?.abort(); this.request?.abort(); this.detailRequest?.abort();
    this.isLoading = false;
    if (this.searchTimer) { clearTimeout(this.searchTimer); this.searchTimer = null; this.offset = 0; }
    const request = this.periodRequest = new AbortController();
    const league = this.session.selectedLeagueName;
    const changed = this.periodLeague !== league;
    if (changed) { this.result = null; this.period = null; this.start = ''; this.end = ''; this.offset = 0; }
    this.periodLoading = true; this.error = '';
    try {
      const period = await this.api.analysisPeriod(league, request.signal);
      if (request.signal.aborted || league !== this.session.selectedLeagueName) return;
      this.period = period; this.periodLeague = league;
      if (this.canAnalyze) {
        const points = observationPoints(period.hours, period.days).filter((point) => point >= this.minimum && point <= this.maximum);
        const savedStart = new Date(this.start).getTime() / 1000, savedEnd = new Date(this.end).getTime() / 1000;
        if (changed || !this.start || !this.end) {
          if (points.length >= 2) this.setRange(points[0]!, points.at(-1)!);
          else { this.start = ''; this.end = ''; }
          this.offset = 0; force = true;
        } else if (!Number.isFinite(savedStart) || !Number.isFinite(savedEnd)
          || savedStart < this.minimum || savedEnd > this.maximum || savedStart >= savedEnd) {
          this.offset = 0; force = true;
        }
      }
      if (this.canAnalyze && this.start && this.end && (force || !this.result || this.loadedLeague !== league)) await this.refresh(false);
      else if (this.result) void this.loadSparklines(this.result);
    } catch (error) {
      if (!request.signal.aborted) { this.period = null; this.result = null; this.error = error instanceof Error ? error.message : String(error); }
    } finally { if (this.periodRequest === request) this.periodLoading = false; }
  }
  private setRange(start: number, end: number): void {
    this.selectedStart = start; this.selectedEnd = end;
    this.start = localInput(start); this.end = localInput(end);
  }
  commitPeriod(range: { start: number; end: number }): void {
    this.setRange(range.start, range.end); void this.refresh();
  }
  searchChangedByUser(): void {
    this.cancelSparklines();
    this.request?.abort();
    if (this.searchTimer) clearTimeout(this.searchTimer);
    this.searchTimer = setTimeout(() => { this.searchTimer = null; void this.refresh(); }, 250);
  }
  async changePage(direction: number): Promise<void> {
    this.offset = Math.max(0, this.offset + direction * 100); await this.refresh(false);
  }
  async analyze(): Promise<void> {
    this.offset = 0;
    await this.datasetChanged();
  }
  async refresh(resetOffset = true): Promise<void> {
    this.cancelSparklines();
    if (this.searchTimer) { clearTimeout(this.searchTimer); this.searchTimer = null; }
    if (!this.period || this.periodLeague !== this.session.selectedLeagueName) { await this.datasetChanged(); return; }
    if (resetOffset) this.offset = 0;
    this.request?.abort(); this.detailRequest?.abort();
    const request = this.request = new AbortController();
    this.isLoading = true; this.error = ''; this.history = null; this.ninjaHistory = null; this.historyRow = '';
    this.excludedLimit = 100;
    try {
      if (!this.canAnalyze) throw new Error('Known league bounds are required before analysis.');
      // Retain absolute timestamps when a band selection falls in a repeated local DST hour.
      const enteredStart = this.start === localInput(this.selectedStart) ? this.selectedStart : new Date(this.start).getTime() / 1000;
      const enteredEnd = this.end === localInput(this.selectedEnd) ? this.selectedEnd : new Date(this.end).getTime() / 1000;
      if (!Number.isFinite(enteredStart) || !Number.isFinite(enteredEnd)
        || enteredStart % 3600 !== 0 || enteredEnd % 3600 !== 0
        || localInput(enteredStart) !== this.start || localInput(enteredEnd) !== this.end) {
        throw new Error('Enter a valid local date and time in yyyy-mm-dd hh:mm format.');
      }
      const start = Math.max(this.minimum, Math.min(this.maximum, enteredStart));
      const end = Math.max(this.minimum, Math.min(this.maximum, enteredEnd));
      this.setRange(start, end);
      if (start >= end) throw new Error('End must be after start.');
      const min = decimal(this.minPrice, 'Minimum starting price'), max = decimal(this.maxPrice, 'Maximum starting price');
      const volume = decimal(this.minVolume, 'Minimum GGG item volume');
      if (min !== undefined && max !== undefined && Number(min) > Number(max)) throw new Error('Minimum starting price must not exceed maximum.');
      const filters: Record<string, string> = { offset: String(this.offset), limit: '100', sort: this.sort,
        search: this.search, category: this.category, venue: this.venue };
      if (min !== undefined) filters.min_price = min;
      if (max !== undefined) filters.max_price = max;
      if (volume !== undefined) filters.min_volume = volume;
      if (this.minConfidence) filters.min_confidence = this.minConfidence;
      if (this.strictStart) filters.strict_start = 'true';
      const league = this.session.selectedLeagueName;
      const result = await this.api.flips(league, start, end, filters, request.signal);
      if (!request.signal.aborted && league === this.session.selectedLeagueName) {
        this.result = result; this.loadedLeague = league; this.offset = result.offset;
        void this.loadSparklines(result);
      }
    } catch (error) {
      if (!request.signal.aborted) { this.result = null; this.error = error instanceof Error ? error.message : String(error); }
    } finally { if (this.request === request) this.isLoading = false; }
  }
}

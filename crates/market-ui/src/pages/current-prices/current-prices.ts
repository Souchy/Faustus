import { resolve } from 'aurelia';
import type {
  EconomyCategory, EconomyHistory, EconomyLeague, EconomyLine, EconomyOverview, EconomySource,
} from '../../core/api-types';
import { MarketApi } from '../../core/market-api';
import { MarketSession } from '../../services/market-session';
import {
  formatNumber, historyDate, historySeries, metadataText, number, quotedPrice, sortedLines, sparkSegments, variantLabel,
  type ChartPoint, type ChartSeries, type EconomySort, type PriceUnit,
} from '../../core/economy';

export class CurrentPrices {
  private readonly api = resolve(MarketApi);
  readonly session = resolve(MarketSession);
  private request: AbortController | null = null;
  private detailRequest: AbortController | null = null;
  private active = false;
  private loadedContext = '';
  get league(): string { return this.session.selectedLeagueId; }
  set league(value: string) { this.session.selectedLeagueId = value; }
  categories: EconomyCategory[] = [];
  get leagues(): EconomyLeague[] { return this.session.datasets; }
  set leagues(value: EconomyLeague[]) { this.session.datasets = value; }
  category = 'Currency';
  source: EconomySource = 'exchange';
  get dataset(): string { return this.session.historicalLeague; }
  set dataset(value: string) { this.session.historicalLeague = value; }
  get date(): string { return this.session.historicalDate; }
  set date(value: string) { this.session.historicalDate = value; }
  dates: string[] = [];
  overview: EconomyOverview | null = null;
  isLoading = false;
  error = '';
  dateError = '';
  search = '';
  unit: PriceUnit = 'native';
  itemType = '';
  corrupted = '';
  gemLevel = '';
  gemQuality = '';
  links = '';
  mapTier = '';
  minListings = '';
  sortColumn: EconomySort = 'price';
  ascending = false;
  limit = 100;
  filteredLines: EconomyLine[] = [];
  rows: EconomyLine[] = [];
  selectedItem: EconomyLine | null = null;
  history: EconomyHistory | null = null;
  historyLoading = false;
  historyError = '';
  historyDays = '';
  historyLimit = 100;
  periodStart = 0;
  periodEnd = 0;
  hoveredCurrency = '';
  hoveredPoint: ChartPoint | null = null;
  private customPeriod: { start: number; end: number } | null = null;
  private dragging: { edge: 'start' | 'end' | 'range'; anchor: number; pointerId: number } | null = null;
  readonly columns: { id: EconomySort; label: string }[] = [
    { id: 'name', label: 'Name' }, { id: 'price', label: 'Value' }, { id: 'change', label: 'Recent trend' },
  ];
  format = formatNumber;
  periodDate = historyDate;
  variant = variantLabel;
  sparks = sparkSegments;
  chartPath(segments: string[]): string {
    return segments.map((segment) => `M${segment.replaceAll(' ', ' L')}${segment.includes(' ') ? '' : ' h0.1'}`).join(' ');
  }

  constructor() {
    const saved = this.session.priceState;
    if (saved) {
      Object.assign(this, saved.values);
      if (saved.dataset !== this.browsingLeague) {
        this.clearFilters();
        this.closeDetail();
      }
    }
  }

  loading(): void {
    this.session.page = 'prices';
    this.session.prices = this;
  }

  attached(): void {
    this.active = true;
    if (!this.overview || this.loadedContext !== this.contextKey) void this.initialize();
    else if (this.selectedItem && !this.history && !this.historyError) void this.loadHistory();
  }

  unbinding(): void {
    this.active = false;
    this.request?.abort();
    this.request = null;
    this.detailRequest?.abort();
    this.detailRequest = null;
    this.isLoading = false;
    this.historyLoading = false;
    this.dragging = null;
    this.hoveredPoint = null;
    this.session.priceState = this.snapshot();
    if (this.session.prices === this) this.session.prices = null;
  }

  snapshot() {
    return {
      dataset: this.browsingLeague,
      values: {
        categories: this.categories, category: this.category, source: this.source,
        dates: this.dates, overview: this.overview, loadedContext: this.loadedContext,
        error: this.error, dateError: this.dateError,
        search: this.search, unit: this.unit, itemType: this.itemType, corrupted: this.corrupted,
        gemLevel: this.gemLevel, gemQuality: this.gemQuality, links: this.links,
        mapTier: this.mapTier, minListings: this.minListings,
        sortColumn: this.sortColumn, ascending: this.ascending, limit: this.limit,
        filteredLines: this.filteredLines, rows: this.rows, selectedItem: this.selectedItem,
        history: this.history, historyError: this.historyError, historyDays: this.historyDays,
        historyLimit: this.historyLimit, periodStart: this.periodStart, periodEnd: this.periodEnd,
        customPeriod: this.customPeriod,
      },
    };
  }

  private controller(): AbortController {
    this.request?.abort();
    const request = new AbortController();
    this.request = request;
    return request;
  }

  async initialize(): Promise<void> {
    await this.session.ensureLoaded();
    if (!this.active) return;
    const request = this.controller();
    this.isLoading = true;
    this.error = '';
    try {
      const categories = await this.api.economyCategories(request.signal);
      if (this.request !== request || !this.active) return;
      this.categories = categories;
      if (!this.categories.length) throw new Error('No economy categories were returned.');
      if (!this.categories.some((category) => category.id === this.category)) this.category = this.categories[0]!.id;
      await this.load();
    } catch (error: unknown) {
      if (this.request === request && this.active) this.error = `Unable to load economy catalog: ${this.message(error)}`;
    } finally {
      if (this.request === request) this.isLoading = false;
    }
  }

  get groups(): { name: string; categories: EconomyCategory[] }[] {
    const groups = new Map<string, EconomyCategory[]>();
    for (const category of this.categories) {
      const entries = groups.get(category.group) ?? [];
      entries.push(category);
      groups.set(category.group, entries);
    }
    return [...groups].map(([name, categories]) => ({ name, categories }));
  }

  get currentCategory(): EconomyCategory | undefined {
    return this.categories.find((category) => category.id === this.category);
  }

  get browsingLeague(): string { return this.dataset || this.league; }
  private get contextKey(): string { return JSON.stringify([this.browsingLeague, this.category, this.source, this.date]); }
  get historical(): boolean {
    return !!this.date || !!this.dataset;
  }
  get availableSources(): EconomySource[] {
    return this.dataset
      ? ['stash'] : this.currentCategory?.sources ?? [];
  }
  get quoteLabel(): string {
    return this.unit === 'native' ? this.overview?.quote_currency ?? '' : this.unit === 'chaos' ? 'Chaos Orb' : 'Divine Orb';
  }
  get conversionUnavailable(): boolean {
    return this.unit !== 'native' && (this.overview?.lines.some((line) => number(line.price) !== null && this.price(line) === null) ?? false);
  }
  get facets(): { field: 'gemLevel' | 'gemQuality' | 'links' | 'mapTier'; label: string; options: number[] }[] {
    const fields = [
      { field: 'gemLevel' as const, key: 'gem_level' as const, label: 'Gem level' },
      { field: 'gemQuality' as const, key: 'gem_quality' as const, label: 'Quality' },
      { field: 'links' as const, key: 'links' as const, label: 'Links' },
      { field: 'mapTier' as const, key: 'map_tier' as const, label: 'Map tier' },
    ];
    return fields.map(({ field, key, label }) => ({ field, label,
      options: [...new Set((this.overview?.lines ?? []).map((line) => line[key]).filter((value) => value !== null))].sort((a, b) => a - b),
    })).filter((facet) => facet.options.length > 0);
  }
  get itemTypes(): string[] {
    return [...new Set((this.overview?.lines ?? []).map((line) => line.item_type).filter((value) => value !== null))].sort();
  }
  get hasListings(): boolean { return this.overview?.lines.some((line) => line.listings !== null) ?? false; }
  get hasCorruption(): boolean { return this.overview?.lines.some((line) => line.corrupted !== null) ?? false; }
  get filterError(): string {
    if (!this.minListings.trim()) return '';
    const value = Number(this.minListings);
    return Number.isSafeInteger(value) && value >= 0 ? '' : 'Minimum listings must be a non-negative whole number.';
  }
  updateLines(): void {
    this.limit = 100;
    if (this.filterError || !this.overview) { this.filteredLines = []; this.rows = []; return; }
    const query = this.search.trim().toLocaleLowerCase();
    const lines = this.overview.lines.filter((line) =>
      (!query || `${line.name} ${line.base_type ?? ''} ${variantLabel(line)}`.toLocaleLowerCase().includes(query))
      && (!this.itemType || line.item_type === this.itemType)
      && (!this.corrupted || line.corrupted === (this.corrupted === 'yes'))
      && (!this.gemLevel || line.gem_level === Number(this.gemLevel))
      && (!this.gemQuality || line.gem_quality === Number(this.gemQuality))
      && (!this.links || line.links === Number(this.links))
      && (!this.mapTier || line.map_tier === Number(this.mapTier))
      && (!this.minListings.trim() || (line.listings !== null && line.listings >= Number(this.minListings))));
    this.filteredLines = sortedLines(lines, this.sortColumn, this.ascending, (line) => this.price(line));
    this.rows = this.filteredLines.slice(0, this.limit);
  }
  showMore(): void { this.limit += 100; this.rows = this.filteredLines.slice(0, this.limit); }
  get historyDates(): string[] {
    return [...new Set((this.history?.points ?? []).map((point) => point.date))].sort();
  }
  get periodPoints() {
    const dates = this.historyDates;
    return (this.history?.points ?? []).filter((point) =>
      point.date >= (dates[this.periodStart] ?? '') && point.date <= (dates[this.periodEnd] ?? ''));
  }
  get series() { return historySeries(this.periodPoints); }
  get navigatorPath(): string { return this.chartPath(historySeries(this.history?.points ?? [])[0]?.segments ?? []); }
  get historyRows() { return [...this.periodPoints].reverse().slice(0, this.historyLimit); }
  hoverHistory(chart: ChartSeries, event: PointerEvent): void {
    if (!(event.currentTarget instanceof SVGSVGElement)) return;
    const matrix = event.currentTarget.getScreenCTM();
    if (!matrix) return;
    const position = new DOMPoint(event.clientX, event.clientY).matrixTransform(matrix.inverse());
    this.setHistoryHover(chart, position.x);
  }
  setHistoryHover(chart: ChartSeries, x: number): void {
    this.hoveredCurrency = chart.currency;
    this.hoveredPoint = chart.points.reduce<ChartPoint | null>((nearest, point) =>
      nearest === null || Math.abs(point.x - x) < Math.abs(nearest.x - x) ? point : nearest, null);
  }
  clearHistoryHover(): void { this.hoveredCurrency = ''; this.hoveredPoint = null; }
  periodPosition(index: number): number {
    const dates = this.historyDates;
    const start = Date.parse(dates[0] ?? '');
    const end = Date.parse(dates.at(-1) ?? '');
    return end > start ? (Date.parse(dates[index] ?? '') - start) / (end - start) * 100 : 0;
  }
  applyHistoryRange(): void {
    this.customPeriod = null;
    this.restoreHistoryRange();
  }
  private restoreHistoryRange(): void {
    this.dragging = null;
    this.clearHistoryHover();
    const dates = this.historyDates;
    this.periodEnd = Math.max(0, dates.length - 1);
    const since = Date.parse(dates.at(-1) ?? '') - Number(this.historyDays) * 86_400_000;
    this.periodStart = this.historyDays ? Math.max(0, dates.findIndex((date) => Date.parse(date) >= since)) : 0;
    if (this.customPeriod && dates.length) {
      this.periodStart = this.nearestPeriodIndex(this.customPeriod.start);
      this.periodEnd = this.nearestPeriodIndex(this.customPeriod.end);
    }
    this.historyLimit = 100;
  }
  resetPeriod(): void { this.historyDays = ''; this.applyHistoryRange(); }
  startPeriodDrag(edge: 'start' | 'end' | 'range', event: PointerEvent): void {
    if (!(event.currentTarget instanceof HTMLElement) || event.button !== 0 || this.dragging || this.historyDates.length < 2) return;
    const graph = event.currentTarget.closest<HTMLElement>('.history-brush');
    const bounds = graph?.getBoundingClientRect();
    if (!graph || !bounds?.width) return;
    const anchor = this.periodIndex((event.clientX - bounds.left) / bounds.width);
    this.dragging = { edge, anchor, pointerId: event.pointerId };
    event.currentTarget.setPointerCapture(event.pointerId);
    if (edge === 'range') this.setPeriodRange(anchor, anchor);
    event.stopPropagation();
    event.preventDefault();
  }
  movePeriodDrag(event: PointerEvent): void {
    if (!this.dragging || this.dragging.pointerId !== event.pointerId || !(event.currentTarget instanceof HTMLElement)) return;
    const bounds = event.currentTarget.getBoundingClientRect();
    if (!bounds.width) return;
    const index = this.periodIndex((event.clientX - bounds.left) / bounds.width);
    if (this.dragging.edge === 'range') this.setPeriodRange(this.dragging.anchor, index);
    else this.setPeriodEdge(this.dragging.edge, index);
  }
  periodIndex(fraction: number): number {
    const dates = this.historyDates;
    if (!dates.length) return 0;
    const start = Date.parse(dates[0]!);
    const target = start + Math.max(0, Math.min(1, fraction)) * (Date.parse(dates.at(-1)!) - start);
    return this.nearestPeriodIndex(target);
  }
  private nearestPeriodIndex(target: number): number {
    const dates = this.historyDates;
    let index = 0;
    dates.forEach((date, candidate) => {
      if (Math.abs(Date.parse(date) - target) < Math.abs(Date.parse(dates[index]!) - target)) index = candidate;
    });
    return index;
  }
  finishPeriodDrag(event: PointerEvent): void {
    this.movePeriodDrag(event);
    this.endPeriodDrag(event);
  }
  endPeriodDrag(event: PointerEvent): void {
    if (this.dragging?.pointerId === event.pointerId) this.dragging = null;
  }
  setPeriodRange(start: number, end: number): void {
    const last = Math.max(0, this.historyDates.length - 1);
    this.periodStart = Math.max(0, Math.min(last, Math.min(start, end)));
    this.periodEnd = Math.max(0, Math.min(last, Math.max(start, end)));
    const dates = this.historyDates;
    if (dates[this.periodStart] && dates[this.periodEnd]) {
      this.customPeriod = { start: Date.parse(dates[this.periodStart]!), end: Date.parse(dates[this.periodEnd]!) };
    }
    this.clearHistoryHover();
    this.historyDays = '';
    this.historyLimit = 100;
  }
  periodKey(edge: 'start' | 'end', event: KeyboardEvent): void {
    if (event.key === 'Enter') { event.preventDefault(); this.resetPeriod(); return; }
    const value = edge === 'start' ? this.periodStart : this.periodEnd;
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? this.historyDates.length - 1
      : event.key === 'ArrowLeft' || event.key === 'ArrowDown' ? value - 1
        : event.key === 'ArrowRight' || event.key === 'ArrowUp' ? value + 1 : null;
    if (next === null) return;
    event.preventDefault();
    this.setPeriodEdge(edge, next);
  }
  setPeriodEdge(edge: 'start' | 'end', index: number): void {
    if (edge === 'start') this.setPeriodRange(Math.min(this.periodEnd, index), this.periodEnd);
    else this.setPeriodRange(this.periodStart, Math.max(this.periodStart, index));
  }
  get iconError(): string {
    return String(this.overview?.lines.find((line) => line.metadata.icon_error)?.metadata.icon_error ?? '');
  }
  get itemMetadata(): { label: string; value: string }[] {
    return Object.entries(this.selectedItem?.metadata ?? {}).map(([label, value]) => ({
      label: label.replace(/([a-z])([A-Z])/g, '$1 $2').replaceAll('_', ' '), value: metadataText(value),
    })).filter(({ value }) => value !== '');
  }

  async refresh(): Promise<void> {
    if (!this.dataset) {
      const request = this.controller();
      this.isLoading = true;
      this.session.error = '';
      const timeout = setTimeout(() => request.abort(new Error('Market refresh request timed out.')), 60_000);
      try {
        await this.session.refreshGemSnapshot(request.signal);
      } catch (error: unknown) {
        if (this.request === request) {
          this.session.error = `Unable to refresh market data: ${this.message(error)}. Check that the external market service is running and reachable.`;
        }
      } finally {
        clearTimeout(timeout);
      }
      if (this.request !== request) return;
    }
    await this.load(true);
  }

  async load(refresh = false): Promise<void> {
    if (!this.browsingLeague) { this.isLoading = false; return; }
    if (!this.categories.length) { await this.initialize(); return; }
    const request = this.controller();
    this.detailRequest?.abort();
    this.selectedItem = null;
    this.overview = null;
    this.updateLines();
    this.isLoading = true;
    this.error = this.dateError = '';
    this.limit = 100;
    if (!this.availableSources.includes(this.source)) this.source = this.availableSources[0] ?? 'stash';
    const timeout = setTimeout(() => request.abort(new Error('Economy request timed out.')), 60_000);
    try {
      const dates = await this.api.economyDates(this.browsingLeague, this.category, this.source, request.signal);
      if (this.request !== request) return;
      this.dates = dates;
      if (this.dataset && !dates.includes(this.date)) {
        this.date = dates[0] ?? '';
      }

      const overview = await this.api.economy(this.browsingLeague, this.category, this.source, this.date, request.signal,
        refresh && !this.historical);
      if (this.request === request) { this.overview = overview; this.loadedContext = this.contextKey; this.updateLines(); }
    } catch (error: unknown) {
      if (this.request === request) this.error = `Unable to load ${this.currentCategory?.name ?? this.category}: ${this.message(error)}`;
    } finally {
      clearTimeout(timeout);
      if (this.request === request) this.isLoading = false;
    }
  }

  selectCategory(category: string): void {
    if (this.category === category) return;
    this.category = category;
    this.clearFilters();
    void this.load();
  }
  reloadCategory(): void { this.clearFilters(); void this.load(); }
  facetValue(field: 'gemLevel' | 'gemQuality' | 'links' | 'mapTier'): string { return this[field]; }
  setFacet(field: 'gemLevel' | 'gemQuality' | 'links' | 'mapTier', value: string): void {
    this[field] = value;
    this.limit = 100;
    this.updateLines();
  }
  selectDataset(dataset: string): void {
    this.dataset = dataset;
  }
  selectSource(source: EconomySource): void {
    this.source = source;
    this.date = '';
    void this.load();
  }
  selectDate(date: string): void { this.date = date; void this.load(); }
  clearFilters(): void {
    this.search = this.itemType = this.corrupted = this.gemLevel = this.gemQuality = this.links = this.mapTier = this.minListings = '';
    this.limit = 100;
    this.updateLines();
  }
  sort(column: EconomySort): void {
    this.ascending = this.sortColumn === column ? !this.ascending : column === 'name';
    this.sortColumn = column;
    this.limit = 100;
    this.updateLines();
  }
  price(line: EconomyLine): number | null { return this.overview ? quotedPrice(line, this.overview, this.unit) : null; }
  trendClass(line: EconomyLine): string { return (number(line.change) ?? 0) < 0 ? 'negative' : 'positive'; }
  safeIcon(icon: string | null): string | null {
    if (!icon) return null;
    try {
      const url = new URL(icon, 'https://web.poecdn.com');
      return url.origin === 'https://web.poecdn.com' && !url.username && !url.password ? url.href : null;
    } catch { return null; }
  }
  timestamp(value: string | undefined): string {
    if (value && /^\d{4}-\d{2}-\d{2}$/.test(value)) return `${value} (daily observation)`;
    return value ? new Date(value).toLocaleString('en-CA', {
      year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hourCycle: 'h23', timeZoneName: 'short',
    }) : 'not available';
  }

  async openItem(line: EconomyLine): Promise<void> {
    this.selectedItem = line;
    this.historyLimit = 100;
    await this.loadHistory();
  }
  closeDetail(): void {
    this.dragging = null;
    this.clearHistoryHover();
    this.detailRequest?.abort();
    this.selectedItem = null;
    this.history = null;
    this.historyError = '';
  }
  async loadHistory(): Promise<void> {
    if (!this.selectedItem) return;
    this.clearHistoryHover();
    this.detailRequest?.abort();
    const request = new AbortController();
    this.detailRequest = request;
    this.history = null;
    this.historyError = '';
    this.historyLoading = true;
    try {
      const history = await this.api.economyHistory(this.browsingLeague, this.category, this.source,
        this.selectedItem.id, '', request.signal);
      if (this.detailRequest === request) { this.history = history; this.restoreHistoryRange(); }
    } catch (error: unknown) {
      if (this.detailRequest === request && !request.signal.aborted) this.historyError = `Unable to load price history: ${this.message(error)}`;
    } finally {
      if (this.detailRequest === request) this.historyLoading = false;
    }
  }
  private message(error: unknown): string { return error instanceof Error ? error.message : String(error); }
}

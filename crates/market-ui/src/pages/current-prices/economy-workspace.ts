import { CustomElement, resolve } from 'aurelia';
import type {
  EconomyArchive, EconomyCategory, EconomyHistory, EconomyLeague, EconomyLine, EconomyOverview, EconomySource,
} from '../../core/api-types';
import { MarketApi } from '../../core/market-api';
import {
  formatNumber, historySeries, metadataText, number, quotedPrice, sortedLines, sparkSegments, variantLabel,
  type EconomySort, type PriceUnit,
} from '../../core/economy';
import template from './economy-workspace.html?raw';

export class EconomyWorkspace {
  private readonly api = resolve(MarketApi);
  private request: AbortController | null = null;
  private detailRequest: AbortController | null = null;
  private archiveRequest: AbortController | null = null;
  private poll: ReturnType<typeof setTimeout> | null = null;
  private active = false;
  league = '';
  refreshToken = 0;
  categories: EconomyCategory[] = [];
  leagues: EconomyLeague[] = [];
  category = 'Currency';
  source: EconomySource = 'exchange';
  dataset = '';
  date = '';
  dates: string[] = [];
  overview: EconomyOverview | null = null;
  loading = false;
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
  managerOpen = false;
  archives: EconomyArchive[] = [];
  archiveLoading = false;
  archiveError = '';
  importing = '';
  notice = '';
  readonly seeds = ['Ancestors', 'Mirage', 'Phrecia 2.0', 'Keepers', 'Mercenaries'];
  readonly columns: { id: EconomySort; label: string }[] = [
    { id: 'name', label: 'Name' }, { id: 'price', label: 'Value' }, { id: 'change', label: 'Recent trend' },
  ];
  format = formatNumber;
  variant = variantLabel;
  sparks = sparkSegments;
  chartPath(segments: string[]): string {
    return segments.map((segment) => `M${segment.replaceAll(' ', ' L')}${segment.includes(' ') ? '' : ' h0.1'}`).join(' ');
  }

  attached(): void {
    this.active = true;
    void this.initialize();
  }

  unbinding(): void {
    this.active = false;
    this.request?.abort();
    this.detailRequest?.abort();
    this.archiveRequest?.abort();
    if (this.poll) clearTimeout(this.poll);
  }

  leagueChanged(): void {
    if (!this.active) return;
    this.dataset = this.date = '';
    this.closeDetail();
    void this.load();
  }

  refreshTokenChanged(): void {
    if (this.active) void this.load(true);
  }

  private controller(): AbortController {
    this.request?.abort();
    const request = new AbortController();
    this.request = request;
    return request;
  }

  async initialize(): Promise<void> {
    const request = this.controller();
    this.loading = true;
    this.error = '';
    try {
      [this.categories, this.leagues] = await Promise.all([
        this.api.economyCategories(request.signal), this.api.economyLeagues(request.signal),
      ]);
      if (this.request !== request || !this.active) return;
      if (!this.categories.length) throw new Error('No economy categories were returned.');
      if (!this.categories.some((category) => category.id === this.category)) this.category = this.categories[0]!.id;
      await this.load();
    } catch (error: unknown) {
      if (this.request === request && this.active) this.error = `Unable to load economy catalog: ${this.message(error)}`;
    } finally {
      if (this.request === request) this.loading = false;
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
  get series() { return historySeries(this.history?.points ?? []); }
  get historyRows() { return [...this.history?.points ?? []].reverse().slice(0, this.historyLimit); }
  get itemMetadata(): { label: string; value: string }[] {
    return Object.entries(this.selectedItem?.metadata ?? {}).map(([label, value]) => ({
      label: label.replace(/([a-z])([A-Z])/g, '$1 $2').replaceAll('_', ' '), value: metadataText(value),
    })).filter(({ value }) => value !== '');
  }

  async load(refresh = false): Promise<void> {
    if (!this.browsingLeague) { this.loading = false; return; }
    if (!this.categories.length) { await this.initialize(); return; }
    const request = this.controller();
    this.detailRequest?.abort();
    this.selectedItem = null;
    this.overview = null;
    this.updateLines();
    this.loading = true;
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
      if (this.request === request) { this.overview = overview; this.updateLines(); }
    } catch (error: unknown) {
      if (this.request === request) this.error = `Unable to load ${this.currentCategory?.name ?? this.category}: ${this.message(error)}`;
    } finally {
      clearTimeout(timeout);
      if (this.request === request) this.loading = false;
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
  browseArchive(archive: EconomyArchive): void {
    this.selectDataset(archive.league ?? archive.name);
    void this.toggleManager();
  }
  selectDataset(dataset: string): void {
    this.dataset = dataset;
    this.date = '';
    this.clearFilters();
    void this.load();
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
    this.detailRequest?.abort();
    this.selectedItem = null;
    this.history = null;
    this.historyError = '';
  }
  async loadHistory(): Promise<void> {
    if (!this.selectedItem) return;
    this.detailRequest?.abort();
    const request = new AbortController();
    this.detailRequest = request;
    this.history = null;
    this.historyError = '';
    this.historyLoading = true;
    try {
      const history = await this.api.economyHistory(this.browsingLeague, this.category, this.source,
        this.selectedItem.id, this.historyDays, request.signal);
      if (this.detailRequest === request) this.history = history;
    } catch (error: unknown) {
      if (this.detailRequest === request && !request.signal.aborted) this.historyError = `Unable to load price history: ${this.message(error)}`;
    } finally {
      if (this.detailRequest === request) this.historyLoading = false;
    }
  }
  async toggleManager(): Promise<void> {
    this.managerOpen = !this.managerOpen;
    if (this.managerOpen) await this.loadArchives();
    else {
      this.archiveRequest?.abort();
      if (this.poll) clearTimeout(this.poll);
    }
  }
  async loadArchives(refresh = false): Promise<void> {
    this.archiveRequest?.abort();
    if (this.poll) clearTimeout(this.poll);
    const request = new AbortController();
    this.archiveRequest = request;
    this.archiveLoading = true;
    this.archiveError = '';
    try {
      const archives = await this.api.economyArchives(request.signal, refresh);
      if (this.archiveRequest !== request) return;
      this.archives = archives;
      this.leagues = await this.api.economyLeagues(request.signal);
      if (archives.some((archive) => ['queued', 'downloading', 'importing'].includes(archive.status))) {
        this.poll = setTimeout(() => { if (this.active && this.managerOpen) void this.loadArchives(); }, 3000);
      }
    } catch (error: unknown) {
      if (this.archiveRequest === request && !request.signal.aborted) this.archiveError = `Unable to load archive status: ${this.message(error)}`;
    } finally {
      if (this.archiveRequest === request) this.archiveLoading = false;
    }
  }
  async importArchive(name: string): Promise<void> {
    this.importing = name;
    this.archiveError = this.notice = '';
    const request = new AbortController();
    try {
      await this.api.importEconomyArchive(name, request.signal);
      this.notice = `Import requested for ${name}. Only softcore challenge data will be imported.`;
      await this.loadArchives();
    } catch (error: unknown) {
      this.archiveError = `Unable to import ${name}: ${this.message(error)}`;
    } finally { this.importing = ''; }
  }
  get allflameAvailable(): boolean { return this.archives.some((archive) => archive.name === 'Allflame'); }
  get seedArchives(): EconomyArchive[] { return this.archives.filter((archive) => this.seeds.includes(archive.name)); }
  private message(error: unknown): string { return error instanceof Error ? error.message : String(error); }
}

CustomElement.define({ name: 'economy-workspace', template, bindables: ['league', 'refreshToken'] }, EconomyWorkspace);

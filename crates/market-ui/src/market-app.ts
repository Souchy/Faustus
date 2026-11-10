import { resolve } from 'aurelia';
import type {
  CurrentExchangePriceOverview, ExchangeMarketSnapshot, GemAnalysisOverview, GemMarketOverview,
  GemSnapshotOption, LeagueOption,
} from './core/api-types';
import {
  decimal, exchangePrice, filterAndSort, gemTypes, opportunities, strategies,
  type GemOpportunity, type GemSortColumn, type GemStrategy, type GemType,
} from './core/gem-analysis';
import { MarketApi } from './core/market-api';

type Page = 'prices' | 'gems' | 'history';

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export class MarketApp {
  private readonly api = resolve(MarketApi);
  private request: AbortController | null = null;
  private gemRequest: AbortController | null = null;
  private gemLeagueId = '';

  leagues: LeagueOption[] = [];
  selectedLeagueId = '';
  page: Page = 'prices';
  loading = false;
  error = '';
  priceError = '';
  gemError = '';
  historyError = '';
  prices: CurrentExchangePriceOverview | null = null;
  gems: GemMarketOverview | null = null;
  gemPrices: CurrentExchangePriceOverview | null = null;
  gemPriceError = '';
  gemSnapshots: GemSnapshotOption[] = [];
  selectedGemSnapshot = 'latest';
  latestGemFetchedAt: string | undefined;
  gemLoading = false;
  markets: ExchangeMarketSnapshot[] = [];
  strategies = strategies;
  strategy: GemStrategy = 'level';
  search = '';
  gemTypes = gemTypes;
  selectedGemTypes: 'all' | GemType[] = 'all';
  gemTypeDropdown: HTMLDetailsElement | null = null;
  includeLowConfidence = false;
  confidenceThresholdText = '10';
  useLens = false;
  lensPriceText = '50';
  sortColumn: GemSortColumn = 'profit';
  sortAscending = false;
  rowLimit = 50;
  rows: GemOpportunity[] = [];
  tableScroll: HTMLElement | null = null;
  readonly columns: { id: GemSortColumn; label: string }[] = [
    { id: 'name', label: 'Gem' }, { id: 'gemType', label: 'Type' },
    { id: 'sourcePrice', label: 'Buy' },
    { id: 'targetPrice', label: 'Sell' },
    { id: 'profit', label: 'Net' }, { id: 'roiPercent', label: 'ROI' },
    { id: 'listings', label: 'Listings' },
  ];

  attached(): void {
    void this.loadDashboard();
  }

  unbinding(): void {
    this.request?.abort();
    this.request = null;
    this.gemRequest?.abort();
    this.gemRequest = null;
  }

  get selectedLeagueName(): string {
    return this.leagues.find((league) => league.id === this.selectedLeagueId)?.name ?? 'the selected league';
  }

  get visibleRows(): GemOpportunity[] {
    return this.rows.slice(0, this.rowLimit);
  }

  get gemTypeSummary(): string {
    const types = this.selectedGemTypes;
    if (types === 'all') return 'All';
    if (!types.length) return 'None';
    return types.length === 1 ? types.join('') : `${types.length} types`;
  }

  get latestGemLabel(): string {
    return this.latestGemFetchedAt
      ? `Latest ${new Date(this.latestGemFetchedAt).toLocaleTimeString('en-CA', {
        hour: '2-digit', minute: '2-digit', hourCycle: 'h23',
      })}` : 'Latest';
  }

  get gcpPrice(): number | null {
    return exchangePrice(this.gemPrices, 'gcp');
  }

  get vaalPrice(): number | null {
    return exchangePrice(this.gemPrices, 'vaal');
  }

  get confidenceThreshold(): number | null {
    const value = Number(this.confidenceThresholdText);
    return this.confidenceThresholdText.trim() !== '' && Number.isSafeInteger(value) && value >= 1 ? value : null;
  }

  get confidenceError(): string {
    return this.confidenceThreshold === null ? 'Enter a whole-number confidence threshold of at least 1.' : '';
  }

  get gemEmptyState(): string {
    if (this.gemError) return this.gemError;
    if (this.loading || this.gemLoading) return `Loading gem variants for ${this.selectedLeagueName}.`;
    if (this.confidenceError) return this.confidenceError;
    if (this.selectedGemTypes !== 'all' && !this.selectedGemTypes.length) return 'Select at least one gem type to show opportunities.';
    if (this.strategy === 'level' && this.useLens && (decimal(this.lensPriceText) ?? 0) <= 0) {
      return "Set a positive Facetor's Lens unit price to calculate this strategy.";
    }
    if (this.strategy === 'quality' && this.gcpPrice === null) return "Gemcutter's Prism prices are unavailable for this instant.";
    if (this.strategy.startsWith('corrupt-') && this.vaalPrice === null) return 'Vaal Orb prices are unavailable for this instant.';
    return 'No matching variants with sufficient confidence and listed prices were found for this strategy.';
  }

  async loadDashboard(preferredLeague = this.selectedLeagueId): Promise<void> {
    this.request?.abort();
    this.gemRequest?.abort();
    this.gemRequest = null;
    this.gemLoading = false;
    const request = new AbortController();
    this.request = request;
    const timeout = setTimeout(() => request.abort(new Error('Market service request timed out.')), 60_000);
    this.loading = true;
    this.error = this.priceError = this.gemError = this.gemPriceError = this.historyError = '';
    this.prices = null;
    this.gems = null;
    this.gemPrices = null;
    this.latestGemFetchedAt = undefined;
    this.markets = [];
    this.updateRows();
    try {
      const [leagues, status] = await Promise.all([
        this.api.leagues(request.signal), this.api.status(request.signal),
      ]);
      if (this.request !== request) return;
      this.leagues = leagues;
      this.selectedLeagueId = leagues.find((league) => league.id === preferredLeague)?.id
        ?? leagues.find((league) => league.id === status.selected_league)?.id
        ?? leagues[0]?.id ?? '';
      if (!this.selectedLeagueId) {
        this.error = 'The market service returned no available leagues.';
        return;
      }
      if (this.gemLeagueId !== this.selectedLeagueId) {
        this.selectedGemSnapshot = 'latest';
        this.gemSnapshots = [];
      }
      this.gemLeagueId = this.selectedLeagueId;
      if (status.selected_league !== this.selectedLeagueId) {
        await this.api.selectLeague(this.selectedLeagueId, request.signal);
      }
      if (this.request !== request) return;
      const [markets, analysis] = await Promise.allSettled([
        this.api.markets(this.selectedLeagueId, request.signal),
        this.api.gemAnalysis(this.selectedLeagueId, 'latest', request.signal),
      ]);
      if (this.request !== request) return;
      if (markets.status === 'fulfilled') this.markets = markets.value;
      else this.historyError = message(markets.reason);
      if (analysis.status === 'fulfilled') {
        this.latestGemFetchedAt = analysis.value.data.gems?.fetched_at;
        this.prices = analysis.value.data.prices;
        this.priceError = analysis.value.data.price_error ?? '';
        if (this.selectedGemSnapshot === 'latest') this.applyGemAnalysis(analysis.value);
        else {
          try {
            const saved = await this.api.gemAnalysis(this.selectedLeagueId, this.selectedGemSnapshot, request.signal);
            if (this.request !== request) return;
            this.applyGemAnalysis(saved);
          } catch (error: unknown) {
            if (this.request !== request) return;
            this.gemError = `Unable to load saved gem snapshot: ${message(error)}. Select Latest to retry current data.`;
          }
        }
      } else {
        this.priceError = this.gemError = message(analysis.reason);
      }
      this.updateRows();
    } catch (error: unknown) {
      if (this.request !== request) return;
      request.abort();
      this.error = `Unable to load market data: ${message(error)}. Check that the external market service is running and reachable.`;
    } finally {
      clearTimeout(timeout);
      if (this.request === request) this.loading = false;
    }
  }

  setPage(page: Page): void {
    this.page = page;
  }

  setStrategy(strategy: GemStrategy): void {
    this.strategy = strategy;
    this.updateRows();
  }

  setSearch(search: string): void {
    this.search = search;
    this.updateRows();
  }

  setGemTypes(types: readonly GemType[]): void {
    this.selectedGemTypes = gemTypes.filter((type) => types.includes(type));
    this.updateRows();
  }

  selectAllGemTypes(): void {
    this.selectedGemTypes = 'all';
    this.updateRows();
  }

  toggleGemType(type: GemType): void {
    const types = this.selectedGemTypes === 'all' ? [] : this.selectedGemTypes;
    this.setGemTypes(types.includes(type) ? types.filter((selected) => selected !== type) : [...types, type]);
  }

  closeGemTypesOutside(event: PointerEvent): void {
    const dropdown = this.gemTypeDropdown;
    if (dropdown?.open && event.target instanceof Node && !dropdown.contains(event.target)) dropdown.open = false;
  }

  closeGemTypesOnBlur(event: FocusEvent): void {
    const dropdown = this.gemTypeDropdown;
    if (dropdown && (!(event.relatedTarget instanceof Node) || !dropdown.contains(event.relatedTarget))) dropdown.open = false;
  }

  onGemTypeKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape' && this.gemTypeDropdown?.open) {
      this.gemTypeDropdown.open = false;
      this.gemTypeDropdown.querySelector('summary')?.focus();
      event.preventDefault();
      event.stopPropagation();
    }
  }

  setLensPrice(price: string): void {
    this.lensPriceText = price;
    this.updateRows();
  }

  toggleLens(): void {
    this.useLens = !this.useLens;
    this.updateRows();
  }

  toggleConfidence(): void {
    this.includeLowConfidence = !this.includeLowConfidence;
    this.updateRows();
  }

  setConfidenceThreshold(value: string): void {
    this.confidenceThresholdText = value;
    this.updateRows();
  }

  async selectGemSnapshot(snapshot: string): Promise<void> {
    this.gemRequest?.abort();
    const request = new AbortController();
    this.gemRequest = request;
    const timeout = setTimeout(() => request.abort(new Error('Gem snapshot request timed out.')), 60_000);
    this.selectedGemSnapshot = snapshot;
    this.gemLoading = true;
    this.gems = this.gemPrices = null;
    this.gemError = this.gemPriceError = '';
    this.updateRows();
    try {
      const analysis = await this.api.gemAnalysis(this.selectedLeagueId, snapshot, request.signal);
      if (this.gemRequest !== request) return;
      this.applyGemAnalysis(analysis);
      if (snapshot === 'latest') {
        this.latestGemFetchedAt = analysis.data.gems?.fetched_at;
        this.prices = analysis.data.prices;
        this.priceError = analysis.data.price_error ?? '';
      }
    } catch (error: unknown) {
      if (this.gemRequest !== request) return;
      this.gemError = `Unable to load gem snapshot: ${message(error)}. Select Latest or refresh to retry.`;
    } finally {
      clearTimeout(timeout);
      if (this.gemRequest === request) {
        this.gemLoading = false;
        this.updateRows();
      }
    }
  }

  private applyGemAnalysis(analysis: GemAnalysisOverview): void {
    this.gems = analysis.data.gems;
    this.gemError = analysis.data.gem_error ?? '';
    this.gemPrices = analysis.data.prices;
    this.gemPriceError = analysis.data.price_error ?? '';
    this.gemSnapshots = analysis.snapshots;
  }

  sort(column: GemSortColumn): void {
    this.sortAscending = this.sortColumn === column ? !this.sortAscending
      : ['name', 'gemType', 'sourceVariant', 'targetVariant'].includes(column);
    this.sortColumn = column;
    this.updateRows();
  }

  updateRows(): void {
    const confidenceThreshold = this.confidenceThreshold;
    this.rows = confidenceThreshold === null ? [] : filterAndSort(opportunities(this.gems?.gems ?? [], this.strategy, {
      gcpPrice: this.gcpPrice, vaalPrice: this.vaalPrice, lensPrice: decimal(this.lensPriceText),
      useLens: this.useLens, confidenceThreshold,
    }), this.search, this.includeLowConfidence, this.sortColumn, this.sortAscending,
    this.selectedGemTypes === 'all' ? gemTypes : this.selectedGemTypes);
    this.rowLimit = 50;
    if (this.tableScroll) this.tableScroll.scrollTop = 0;
  }

  showMore(): void {
    this.rowLimit += 50;
  }

  money(value: number | null): string {
    return value === null ? 'not available' : `${value.toFixed(2)}c`;
  }

  timestamp(value: string | undefined): string {
    return value ? new Date(value).toLocaleString('en-CA', {
      year: 'numeric', month: '2-digit', day: '2-digit',
      hour: '2-digit', minute: '2-digit', hourCycle: 'h23', timeZoneName: 'short',
    }) : 'not available';
  }

  snapshotLabel(value: string): string {
    return new Date(value).toLocaleString('en-CA', {
      month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hourCycle: 'h23',
    });
  }

  shortId(value: string): string {
    return value.split('/').at(-1) ?? value;
  }

  pair(values: string[]): string {
    return values.map((value) => this.shortId(value)).join(' / ');
  }

  sideValues(values: Record<string, number>): string {
    return Object.entries(values).sort(([a], [b]) => a.localeCompare(b))
      .map(([item, amount]) => `${this.shortId(item)} ${amount}`).join(', ');
  }
}

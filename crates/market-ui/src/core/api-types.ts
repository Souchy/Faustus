// Mirrors the service JSON DTOs; decimal prices remain strings on the wire.
export interface LeagueOption {
  id: string;
  name: string;
}

export interface ServiceStatus {
  exchange_cursor: number | null;
  exchange_cursor_at: string | null;
  exchange_pages: number;
  exchange_markets: number;
  ninja_snapshots: number;
  selected_league: string | null;
}

export interface ExchangeMarketSnapshot {
  league: string;
  market_id: string;
  market_pair: string[];
  volume_traded: Record<string, number>;
  lowest_stock: Record<string, number>;
  highest_stock: Record<string, number>;
  lowest_ratio: Record<string, number>;
  highest_ratio: Record<string, number>;
  next_change_id: number;
}

export interface ExchangePriceCurrency {
  id: string;
  name: string;
  image: string | null;
}

export interface CurrentExchangePriceLine {
  id: string;
  name: string;
  item_category: string | null;
  image: string | null;
  primary_value: string | null;
  volume_primary_value: string | null;
  max_volume_currency: string | null;
  max_volume_rate: string | null;
  sparkline_change: string | null;
  sparkline: (string | null)[];
}

export interface CurrentExchangePriceOverview {
  league: string;
  category: string;
  fetched_at: string;
  primary: ExchangePriceCurrency;
  secondary: ExchangePriceCurrency | null;
  lines: CurrentExchangePriceLine[];
}

export interface GemMarketLine {
  name: string;
  gem_kind: string | null;
  variant: string | null;
  gem_level: number | null;
  gem_quality: number | null;
  corrupted: boolean;
  chaos_value: string | null;
  sample_count: number | null;
  listing_count: number | null;
  sparkline_change: string | null;
  sparkline: (string | null)[];
}

export interface GemMarketOverview {
  league: string;
  fetched_at: string;
  gems: GemMarketLine[];
}

export interface GemAnalysisData {
  gems: GemMarketOverview | null;
  gem_error: string | null;
  prices: CurrentExchangePriceOverview | null;
  price_error: string | null;
}

export interface GemSnapshotOption {
  id: number;
  captured_at: string;
}

export interface GemAnalysisOverview {
  snapshot: GemSnapshotOption | null;
  data: GemAnalysisData;
  snapshots: GemSnapshotOption[];
}

export type EconomySource = 'exchange' | 'stash';

export interface EconomyCategory {
  id: string;
  name: string;
  group: string;
  sources: EconomySource[];
}

export interface EconomyLeague extends LeagueOption {
  live: boolean;
  archived?: boolean;
}

export interface EconomyLine {
  id: string;
  name: string;
  icon: string | null;
  details_id: string | null;
  price: string | null;
  divine_price: string | null;
  change: string | null;
  sparkline: (string | null)[];
  volume: string | null;
  pair: string | null;
  pair_rate: string | null;
  listings: number | null;
  samples: number | null;
  variant: string | null;
  base_type: string | null;
  item_type: string | null;
  links: number | null;
  gem_level: number | null;
  gem_quality: number | null;
  corrupted: boolean | null;
  map_tier: number | null;
  metadata: Record<string, unknown>;
}

export interface EconomyOverview {
  league: string;
  category: string;
  source: EconomySource;
  fetched_at: string;
  quote_currency: string;
  rates: Record<string, string>;
  lines: EconomyLine[];
}

export interface EconomyHistoryPoint {
  date: string;
  price: string;
  currency: string;
  granularity: 'daily' | 'snapshot';
  volume?: string | null;
}

export interface EconomyHistory {
  points: EconomyHistoryPoint[];
  coverage: { start: string | null; end: string | null };
}

export interface EconomyArchive {
  name: string;
  status: 'available' | 'queued' | 'downloading' | 'importing' | 'completed' | 'failed';
  rows: number;
  skipped: number;
  downloaded_bytes: number;
  download_total_bytes: number | null;
  total_rows: number | null;
  error: string | null;
  start: string | null;
  end: string | null;
  league: string | null;
  progress: string;
  license: string;
  url: string;
  ggg?: GggArchiveImport | null;
  ggg_error?: string | null;
}
export interface GggItem { id: string; name: string; category: string; icon: string | null }
export interface GggMarket {
  id: string; a: GggItem; b: GggItem; volume_a: string; volume_b: string;
  low_stock_a: string; low_stock_b: string; high_stock_a: string; high_stock_b: string;
  low_ratio_a: string; low_ratio_b: string; high_ratio_a: string; high_ratio_b: string;
}
export interface GggCoverage {
  league: string; start: number; end: number; expected_hours: number; completed_hours: number[];
  failed_hours: { hour: number; error: string }[];
}
export interface GggOverview { league: string; hour: number; complete: boolean; markets: GggMarket[]; platform?: 'PC' }
export interface GggHistory {
  league: string; market: string; points: { hour: number; market: GggMarket }[]; coverage: GggCoverage;
}
export interface GggImportJob {
  league: string; start: number; end: number; status: 'running' | 'completed' | 'cancelled' | 'failed';
  completed_hours: number; total_hours: number; error: string | null;
  warning?: string | null;
  platform?: 'PC';
}
export interface GggArchiveImport extends Omit<GggImportJob, 'status'> {
  status: GggImportJob['status'] | 'available' | 'partial';
}
export interface PairAlternative {
  start_observed_at?: string; end_observed_at?: string;
  market: string; quote: string; native_start: string; native_end: string; start_price: string;
  end_price: string; growth: string; growth_percent: string; start_volume: string; end_volume: string; selected: boolean;
}
export interface FlipRow {
  id: string; name: string; category: string; venue: 'exchange' | 'trade'; source: string;
  start_price: string; end_price: string; growth: string; growth_percent: string;
  start_observed_at: string; end_observed_at: string; pair: string; alternatives: PairAlternative[];
  native_start?: string; native_end?: string;
  history_category?: string | null; history_source?: string | null; history_item?: string | null;
  start_confidence?: string | null; end_confidence?: string | null;
  history_archive?: string | null; history_granularity?: string | null;
}
export interface FlipSparklinePoint { time: number; price: string | null }
export interface FlipSparklineSeries { id: string; points: FlipSparklinePoint[]; warnings: string[] }
export interface FlipSparklines { series: FlipSparklineSeries[] }
export interface FlipAnalysis {
  league: string; start: number; end: number; rows: FlipRow[];
  excluded: { name: string; reason: string }[]; warnings: string[];
  total: number; offset: number; limit: number; excluded_total: number; categories: string[];
}
export interface AnalysisPeriod {
  league: string; start: number | null; end: number | null;
  bounds_source: 'official' | 'archive' | 'observed' | 'unavailable';
  hours: number[]; days: string[]; warnings: string[];
}
export interface ArbitrageLoop {
  id: string; path: string[]; rates: string[]; market_ids: string[]; return_percent: string;
  start_amount: string; end_amount: string; minimum_volume: string;
}
export interface ArbitrageAnalysis {
  league: string; hour: number; complete: boolean; loops: ArbitrageLoop[]; warnings: string[];
}

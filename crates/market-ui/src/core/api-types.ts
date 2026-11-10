// Mirrors the JSON DTOs in market-core; decimal prices remain strings on the wire.
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

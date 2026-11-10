import type {
  CurrentExchangePriceLine, CurrentExchangePriceOverview, GemAnalysisOverview, GemMarketLine,
  GemMarketOverview, ServiceStatus,
} from '../core/api-types';

export function gem(level: number, quality: number, corrupted: boolean, value: string): GemMarketLine {
  return {
    name: 'Fireball', gem_kind: null, variant: `${level}/${quality}`, gem_level: level, gem_quality: quality,
    corrupted, chaos_value: value, sample_count: 12, listing_count: 12,
    sparkline_change: '0', sparkline: Array<string>(7).fill('0'),
  };
}

export function gemOverview(): GemMarketOverview {
  return {
    league: 'Standard', fetched_at: '2026-11-09T12:00:00Z',
    gems: [gem(1, 0, false, '2'), gem(20, 0, false, '40'), gem(20, 20, false, '100'),
      gem(21, 20, true, '140'), gem(20, 23, true, '120')],
  };
}

export function categorizedGemOverview(): GemMarketOverview {
  const categories: { name: string; kind: string | null }[] = [
    { name: 'Fireball', kind: null },
    { name: 'Multistrike Support', kind: null },
    { name: 'Empower Support', kind: 'exceptional' },
    { name: 'Cleave of Rage', kind: 'transfigured' },
    { name: 'Sunder of Trarthus', kind: 'trarthus' },
  ];
  return {
    ...gemOverview(),
    gems: categories.flatMap(({ name, kind }) =>
      gemOverview().gems.map((gem) => ({ ...gem, name, gem_kind: kind }))),
  };
}

export function prices(): CurrentExchangePriceOverview {
  const line = (id: string, name: string, value: string): CurrentExchangePriceLine => ({
    id, name, item_category: null, image: null, primary_value: value, volume_primary_value: '100',
    max_volume_currency: 'Divine Orb', max_volume_rate: '2', sparkline_change: '0',
    sparkline: Array<string>(7).fill('0'),
  });
  return {
    league: 'Standard', category: 'Currency', fetched_at: '2026-11-09T12:00:00Z',
    primary: { id: 'chaos', name: 'Chaos Orb', image: null }, secondary: null,
    lines: [line('gcp', "Gemcutter's Prism", '1'), line('vaal', 'Vaal Orb', '2')],
  };
}

export function status(selectedLeague: string | null = 'Standard'): ServiceStatus {
  return {
    exchange_cursor: null, exchange_cursor_at: null, exchange_pages: 0,
    exchange_markets: 0, ninja_snapshots: 0, selected_league: selectedLeague,
  };
}

export function gemAnalysis(gems = gemOverview(), saved = false): GemAnalysisOverview {
  const snapshot = { id: saved ? 1 : 2, captured_at: saved ? '2026-11-08T12:00:00Z' : '2026-11-09T12:00:00Z' };
  const materialPrices = prices();
  if (saved) {
    gems = { ...gems, fetched_at: snapshot.captured_at,
      gems: gems.gems.map((gem) => ({ ...gem, chaos_value: String(Number(gem.chaos_value) / 2),
        sample_count: 10, listing_count: 10 })) };
    materialPrices.fetched_at = snapshot.captured_at;
    materialPrices.lines = materialPrices.lines.map((line) => ({ ...line, primary_value: '0.5' }));
  }
  return {
    snapshot, data: { gems, gem_error: null, prices: materialPrices, price_error: null },
    snapshots: [{ id: 2, captured_at: '2026-11-09T12:00:00Z' }, { id: 1, captured_at: '2026-11-08T12:00:00Z' }],
  };
}

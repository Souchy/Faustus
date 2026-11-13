import type { AnalysisPeriod, ArbitrageAnalysis, FlipAnalysis, FlipRow, FlipSparklines, GggCoverage, GggHistory, GggImportJob, GggMarket, GggOverview } from '../core/api-types';

export const historicalHour = 1794412800;
export function analysisPeriod(league = 'Standard'): AnalysisPeriod {
  const end = league === 'Mirage' ? Date.parse('2026-03-13T00:00:00Z') / 1000 : historicalHour;
  const start = end - (league === 'Mirage' ? 4 : 1) * 86400;
  return { league, start, end, bounds_source: 'archive', hours: [start, start + 6 * 3600, end - 6 * 3600, end],
    days: [new Date(start * 1000).toISOString().slice(0, 10), new Date(end * 1000).toISOString().slice(0, 10)],
    warnings: ['Archive-derived bounds; official league dates unavailable.'] };
}
export function gggMarket(): GggMarket {
  return {
    id: 'chaos|divine',
    a: { id: 'chaos', name: 'Chaos Orb', category: 'Currency', icon: null },
    b: { id: 'divine', name: 'Divine Orb', category: 'Currency', icon: null },
    volume_a: '1000', volume_b: '5', low_stock_a: '1', low_stock_b: '1',
    high_stock_a: '1000', high_stock_b: '5', low_ratio_a: '200', low_ratio_b: '1',
    high_ratio_a: '200', high_ratio_b: '1',
  };
}
export function gggCoverage(start = historicalHour - 23 * 3600, end = historicalHour): GggCoverage {
  return { league: 'Standard', start, end, expected_hours: (end - start) / 3600 + 1,
    completed_hours: [start, end], failed_hours: [{ hour: start + 3600, error: 'Upstream HTTP 503' }] };
}
export function gggOverview(hour = historicalHour): GggOverview {
  return { league: 'Standard', hour, complete: true, markets: [gggMarket()] };
}
export function gggHistory(start = historicalHour - 23 * 3600, end = historicalHour): GggHistory {
  return { league: 'Standard', market: gggMarket().id, coverage: gggCoverage(start, end),
    points: [{ hour: start, market: gggMarket() }, { hour: end, market: { ...gggMarket(), volume_b: '6' } }] };
}
export function gggJob(status: GggImportJob['status'] = 'running'): GggImportJob {
  return { league: 'Standard', start: historicalHour - 23 * 3600, end: historicalHour, status,
    completed_hours: status === 'completed' ? 24 : 2, total_hours: 24, error: null };
}
export function flipAnalysis(start = historicalHour - 24 * 3600, end = historicalHour): FlipAnalysis {
  const row = {
    id: 'divine', name: 'Divine Orb', category: 'Currency', venue: 'exchange' as const, source: 'GGG Exchange',
    start_price: '100', end_price: '120', growth: '20', growth_percent: '20',
    native_start: '100', native_end: '120',
    start_observed_at: new Date(start * 1000).toISOString(), end_observed_at: new Date(end * 1000).toISOString(),
    pair: 'divine|chaos', alternatives: [{
      market: 'divine|chaos', quote: 'Chaos Orb', native_start: '100', native_end: '120', start_price: '100',
      end_price: '120', growth: '20', growth_percent: '20', start_volume: '50', end_volume: '60', selected: true,
    }],
  };
  return { league: 'Standard', start, end, total: 2, offset: 0, limit: 100, excluded_total: 1, categories: ['Currency', 'Oils'], rows: [row, { ...row, id: 'oil', name: 'Golden Oil', category: 'Oils',
    source: 'poe.ninja dump', venue: 'trade', start_price: '10', end_price: '15', growth: '5', growth_percent: '50',
    pair: 'Chaos Orb', alternatives: [], native_start: '10', native_end: '15',
    history_category: 'Oil', history_source: 'stash', history_item: 'golden-oil',
    history_archive: 'Standard', history_granularity: 'daily', start_confidence: 'High', end_confidence: 'Low' }],
  warnings: ['Only saved in-period starts and selected-end observations are ranked.'], excluded: [{ name: 'Silver Oil', reason: 'Missing end observation' }] };
}
export function flipSparklines(rows: FlipRow[], start = historicalHour - 86400, end = historicalHour): FlipSparklines {
  const span = end - start;
  return { series: rows.map((row) => ({ id: row.id, warnings: ['At most 60 evenly spaced saved hours/days; gaps at sampled observations are not interpolated. Unsampled fluctuations and gaps are not represented.'], points: [
    { time: start, price: row.start_price },
    { time: start + span / 4, price: row.start_price },
    { time: start + span / 2, price: null },
    { time: start + span * 3 / 4, price: row.end_price },
    { time: end, price: row.end_price },
  ] })) };
}
export function arbitrageAnalysis(hour = historicalHour): ArbitrageAnalysis {
  return { league: 'Standard', hour, complete: false, warnings: ['Missing native market observations.'], loops: [{
    id: 'loop', path: ['CHAOS', 'DIVINE', 'EXALTED', 'CHAOS'], rates: ['0.005', '100', '2.1'],
    market_ids: ['chaos|divine', 'divine|exalted', 'exalted|chaos'], return_percent: '5',
    start_amount: '1', end_amount: '1.05', minimum_volume: '100',
  }] };
}

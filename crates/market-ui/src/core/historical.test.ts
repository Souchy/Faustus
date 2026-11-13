import { afterEach, describe, expect, it, vi } from 'vitest';
import { decimal, latestHour, localInput, meanRatio, nativeRatioLabel, parseHour, parseRange, priceSegments } from './historical';
import { gggHistory, gggMarket } from '../testing/historical-fixtures';

afterEach(() => vi.useRealTimers());
describe('Historical transformations', () => {
  it('round trips a local control to the exact UTC hour and rejects partial/current hours', () => {
    vi.useFakeTimers().setSystemTime(new Date('2026-11-12T05:30:00Z'));
    const hour = latestHour();
    expect(hour).toBe(Date.parse('2026-11-12T04:00:00Z') / 1000);
    expect(parseHour(localInput(hour))).toBe(hour);
    expect(() => parseHour(localInput(hour + 3600))).toThrow('current hour');
    expect(() => parseHour(localInput(hour).slice(0, -2) + '01')).toThrow('exact UTC hour');
  });
  it('validates inclusive 180-day import and distinct analysis ranges without sampling', () => {
    const end = latestHour(), start = end - 4319 * 3600;
    expect(parseRange(localInput(start), localInput(end))).toEqual([start, end]);
    expect(() => parseRange(localInput(start - 3600), localInput(end))).toThrow('180 days');
    expect(() => parseRange(localInput(end), localInput(end), true)).toThrow('after start');
  });
  it('accepts zero and plain decimals only', () => {
    expect(decimal('', 'Volume')).toBeUndefined();
    expect(decimal('0', 'Volume')).toBe('0');
    expect(decimal('.125', 'Volume')).toBe('.125');
    for (const value of ['-1', 'Infinity', 'NaN', '1e3', '1,000']) expect(() => decimal(value, 'Volume')).toThrow('nonnegative');
  });
  it('uses native traded amounts for means and never joins missing or zero-trade hours', () => {
    expect(meanRatio(gggMarket())).toBe(0.005);
    expect(meanRatio({ ...gggMarket(), volume_a: '0' })).toBeNull();
    const history = gggHistory();
    expect(priceSegments(history)).toHaveLength(2);
    expect(priceSegments(history).every((p) => !p.includes(' L'))).toBe(true);
    history.points = [
      { hour: history.coverage.start, market: gggMarket() },
      { hour: history.coverage.start + 3600, market: gggMarket() },
      { hour: history.coverage.start + 7200, market: { ...gggMarket(), volume_b: '0' } },
      { hour: history.coverage.start + 10800, market: gggMarket() },
    ];
    expect(priceSegments(history)).toHaveLength(2);
    expect(priceSegments(history)[0]).toContain(' L');
  });
  it('inverts sub-unit native history ratios and their units without changing larger ratios or missing prices', () => {
    const market = { ...gggMarket(), a: { ...gggMarket().a, name: 'Chaos Orb' },
      b: { ...gggMarket().b, name: 'The Chosen' }, volume_a: '2', volume_b: '1' };
    expect(nativeRatioLabel(market)).toBe('2 Chaos Orb per The Chosen');
    expect(nativeRatioLabel({ ...market, volume_a: '4' })).toBe('4 Chaos Orb per The Chosen');
    expect(nativeRatioLabel({ ...market, volume_a: '61', volume_b: '4.5' })).toBe('13,56 Chaos Orb per The Chosen');
    expect(nativeRatioLabel({ ...market, volume_a: '1', volume_b: '1' })).toBe('1 The Chosen per Chaos Orb');
    const lifeforce = { ...market, b: { ...market.b, name: 'Wild Crystallised Lifeforce' },
      volume_a: '1', volume_b: '90' };
    expect(nativeRatioLabel(lifeforce)).toBe('90 Wild Crystallised Lifeforce per Chaos Orb');
    expect(nativeRatioLabel({ ...lifeforce, volume_a: '100', volume_b: '6473' })).toBe('64,73 Wild Crystallised Lifeforce per Chaos Orb');
    expect(nativeRatioLabel({ ...market, volume_a: '0' })).toBe('- The Chosen per Chaos Orb');
    expect(nativeRatioLabel({ ...market, volume_b: '0' })).toBe('- The Chosen per Chaos Orb');
    expect(meanRatio(market)).toBe(0.5);
  });
});

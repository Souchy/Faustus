import { describe, expect, it } from 'vitest';
import { formatNumber, historySeries, metadataText, number, quotedPrice, sortedLines, sparkSegments, variantLabel } from './economy';
import { economyLine, economyOverview } from '../testing/fixtures';

describe('economy values and charts', () => {
  it('converts only with rates from the overview and does not invent missing or invalid values', () => {
    const overview = economyOverview();
    const line = economyLine({ price: '400' });
    expect(quotedPrice(line, overview, 'divine')).toBe(2);
    expect(quotedPrice(line, overview, 'chaos')).toBe(400);
    overview.rates = {};
    expect(quotedPrice(line, overview, 'divine')).toBeNull();
    expect(quotedPrice({ ...line, divine_price: '3' }, overview, 'divine')).toBe(3);
    expect(number(null)).toBeNull();
    expect(number('Infinity')).toBeNull();
    expect(number('')).toBeNull();
    expect(number('0')).toBe(0);
    expect(formatNumber('0.00000123')).toBe('0.00000123');
    expect(metadataText([{ text: '10% increased damage' }])).toBe('10% increased damage');
  });

  it('keeps variant names distinct and sorts nulls last in both directions', () => {
    const lines = [
      economyLine({ id: 'a', price: null }), economyLine({ id: 'b', price: '4' }), economyLine({ id: 'c', price: '2' }),
    ];
    expect(sortedLines(lines, 'price', true, (line) => number(line.price)).map((line) => line.id)).toEqual(['c', 'b', 'a']);
    expect(sortedLines(lines, 'price', false, (line) => number(line.price)).map((line) => line.id)).toEqual(['b', 'c', 'a']);
    expect(variantLabel(economyLine({ gem_level: 21, gem_quality: 20, corrupted: true })))
      .toContain('Level 21 · 20% quality · Corrupted');
    expect(lines[0]?.price).toBeNull();
  });

  it('breaks sparkline gaps and separates daily/snapshot series without fabricating timestamps', () => {
    expect(sparkSegments(['0', '1', null, '3'])).toHaveLength(2);
    expect(sparkSegments([null])).toEqual([]);
    const series = historySeries([
      { date: '2026-01-01', price: '1', currency: 'Chaos Orb', granularity: 'daily' },
      { date: '2026-01-02', price: '2', currency: 'Chaos Orb', granularity: 'daily' },
      { date: '2026-01-05', price: '3', currency: 'Chaos Orb', granularity: 'daily' },
      { date: '2026-01-05T00:15:00Z', price: '4', currency: 'Chaos Orb', granularity: 'snapshot' },
      { date: '2026-01-05T00:15:00Z', price: '1', currency: 'Divine Orb', granularity: 'snapshot' },
    ]);
    expect(series).toHaveLength(2);
    expect(series[0]?.segments).toHaveLength(3);
    expect(series[0]?.points).toHaveLength(4);
    expect(series[0]?.points[0]?.x).toBe(16);
    expect(series[0]?.points.at(-1)?.x).toBe(784);
  });
});

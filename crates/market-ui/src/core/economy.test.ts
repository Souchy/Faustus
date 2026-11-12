import { describe, expect, it } from 'vitest';
import { formatNumber, historyDate, historySeries, metadataText, number, quotedPrice, sortedLines, sparkSegments, variantLabel } from './economy';
import { economyLine, economyOverview } from '../testing/fixtures';

describe('economy values and charts', () => {
  it('uses spaces for thousands and decimal commas while preserving chart tick precision', () => {
    expect(formatNumber('1234567.89')).toBe('1 234 567,89');
    expect(formatNumber('-12345.67')).toBe('-12 345,67');
    expect(formatNumber('1000')).toBe('1 000');
    expect(formatNumber('1234.56789', { maximumFractionDigits: 5 })).toBe('1 234,56789');
    expect(formatNumber('0.00000123')).toBe('0,00000123');
    expect(formatNumber(null)).toBe('-');
    const chart = historySeries([
      { date: '2026-01-01', price: '10000', currency: 'Chaos Orb', granularity: 'daily' },
      { date: '2026-01-02', price: '20000', currency: 'Chaos Orb', granularity: 'daily' },
    ])[0]!;
    expect(chart.priceTicks.map((tick) => tick.label)).toEqual(['10 000', '12 500', '15 000', '17 500', '20 000']);
    expect(chart.points[0]?.label).toBe('2026-01-01: 10 000 Chaos Orb (daily)');
  });

  it('builds adaptive price ticks, day ticks, visible dots and aligned volume without filling gaps', () => {
    const points = [
      { date: '2026-01-01', price: '12', volume: '40', currency: 'Chaos Orb', granularity: 'daily' as const },
      { date: '2026-01-02', price: '18', volume: null, currency: 'Chaos Orb', granularity: 'daily' as const },
      { date: '2026-01-03', price: '20', volume: '0', currency: 'Chaos Orb', granularity: 'daily' as const },
    ];
    const chart = historySeries(points)[0]!;
    expect(chart.priceTicks.map((tick) => tick.label)).toEqual(['12', '14', '16', '18', '20']);
    expect(chart.priceTicks.map((tick) => tick.position)).toEqual([90, 70, 50, 30, 10]);
    expect(chart.dayTicks.map((tick) => tick.label)).toEqual(['Jan 1', 'Jan 2', 'Jan 3']);
    expect(chart.dots.match(/M/g)).toHaveLength(3);
    expect(chart.grid).toContain('M16,180 H784');
    expect(chart.volume).toEqual({ bars: 'M16,56 V4 M784,56 V56', count: 2, max: 40 });
    const selected = historySeries(points.slice(1))[0]!;
    expect(selected.min).toBe(18);
    expect(selected.priceTicks[0]?.label).toBe('18');
    expect(selected.volume).toEqual({ bars: 'M784,56 V56', count: 1, max: 0 });
    expect(historySeries([{ ...points[0]!, volume: undefined }])[0]?.volume.count).toBe(0);
    expect(historyDate('2026-01-01T12:15:46.123456Z')).toBe('2026-01-01');
  });

  it('keeps constant, zero and fractional scales finite and limits long-range day labels', () => {
    for (const price of ['0', '200', '0.000002']) {
      const chart = historySeries([{ date: '2026-01-01', price, currency: 'Chaos Orb', granularity: 'daily' }])[0]!;
      expect(Number.isFinite(chart.points[0]?.y)).toBe(true);
      expect(chart.priceTicks.length).toBeGreaterThanOrEqual(3);
      expect(new Set(chart.priceTicks.map((tick) => tick.label)).size).toBe(chart.priceTicks.length);
      expect(chart.dayTicks).toHaveLength(1);
      expect(chart.dots).not.toContain('NaN');
    }
    const chart = historySeries(Array.from({ length: 60 }, (_, index) => ({
      date: new Date(Date.UTC(2026, 0, index + 1)).toISOString(), price: String(index),
      currency: 'Chaos Orb', granularity: 'snapshot' as const,
    })))[0]!;
    expect(chart.dayTicks).toHaveLength(5);
    expect(chart.points[0]?.label).toBe('2026-01-01 00:00 UTC: 0 Chaos Orb (snapshot)');
  });

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
    expect(formatNumber('0.00000123')).toBe('0,00000123');
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

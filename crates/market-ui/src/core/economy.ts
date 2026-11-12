import type { EconomyHistoryPoint, EconomyLine, EconomyOverview } from './api-types';

export type EconomySort = 'name' | 'price' | 'change' | 'volume' | 'listings';
export type PriceUnit = 'native' | 'chaos' | 'divine';
export interface ChartPoint { x: number; y: number; label: string }
export interface ChartTick { position: number; label: string }
export interface ChartSeries {
  currency: string; segments: string[]; points: ChartPoint[]; min: number; max: number;
  priceTicks: ChartTick[]; dayTicks: ChartTick[]; grid: string; dots: string;
  volume: { bars: string; count: number; max: number };
}

export function historyDate(value: string | undefined): string {
  return value?.slice(0, 10) ?? '';
}

function priceScale(min: number, max: number): { min: number; max: number; ticks: ChartTick[] } {
  const padding = min === max ? Math.abs(min) * .05 || 1 : 0;
  const low = Math.max(0, min - padding);
  const high = max + padding;
  const rawStep = (high - low) / 4;
  const magnitude = 10 ** Math.floor(Math.log10(rawStep));
  const step = ([1, 2, 2.5, 5, 10].find((multiple) => multiple * magnitude >= rawStep) ?? 10) * magnitude;
  const start = Math.floor(low / step) * step;
  const end = Math.ceil(high / step) * step;
  const ticks = Array.from({ length: Math.round((end - start) / step) + 1 }, (_, index) => {
    const value = start + index * step;
    return {
      position: (180 - (value - start) / (end - start) * 160) / 2,
      label: formatNumber(value, { maximumFractionDigits: Math.min(20, Math.max(0, 1 - Math.floor(Math.log10(step)))) }),
    };
  });
  return { min: start, max: end, ticks };
}

export function number(value: string | number | null | undefined): number | null {
  if (value === null || value === undefined || String(value).trim() === '') return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

export function formatNumber(value: string | number | null | undefined, options?: Intl.NumberFormatOptions): string {
  const parsed = number(value);
  return parsed === null ? '-' : parsed.toLocaleString('en-CA',
    options ?? (parsed !== 0 && Math.abs(parsed) < 1 ? { maximumSignificantDigits: 4 } : { maximumFractionDigits: 2 }))
    .replaceAll(',', ' ').replace('.', ',');
}

export function variantLabel(line: EconomyLine): string {
  return [
    line.variant,
    line.gem_level != null ? `Level ${line.gem_level}` : '',
    line.gem_quality != null ? `${line.gem_quality}% quality` : '',
    line.links != null && line.links > 0 ? `${line.links} links` : '',
    line.map_tier != null ? `Tier ${line.map_tier}` : '',
    line.corrupted ? 'Corrupted' : '',
  ].filter(Boolean).join(' · ');
}

export function quotedPrice(line: EconomyLine, overview: EconomyOverview, unit: PriceUnit): number | null {
  const value = number(line.price);
  if (unit === 'native' || value === null) return value;
  if (unit === 'divine' && number(line.divine_price) !== null) return number(line.divine_price);
  const target = unit === 'chaos' ? 'Chaos Orb' : 'Divine Orb';
  if (overview.quote_currency === target) return value;
  const rate = number(overview.rates[target]);
  return rate !== null && rate > 0 ? value / rate : null;
}

export function sortedLines(lines: EconomyLine[], sort: EconomySort, ascending: boolean,
  price: (line: EconomyLine) => number | null): EconomyLine[] {
  return [...lines].sort((a, b) => {
    if (sort === 'name') return a.name.localeCompare(b.name) * (ascending ? 1 : -1) || a.id.localeCompare(b.id);
    const left = sort === 'price' ? price(a) : number(a[sort]);
    const right = sort === 'price' ? price(b) : number(b[sort]);
    if (left === null || right === null) return left === right ? a.id.localeCompare(b.id) : left === null ? 1 : -1;
    return (left - right) * (ascending ? 1 : -1) || a.name.localeCompare(b.name) || a.id.localeCompare(b.id);
  });
}

export function sparkSegments(samples: (string | null)[], width = 110, height = 28): string[] {
  const valid = samples.map(number).filter((value) => value !== null);
  if (!valid.length) return [];
  const min = Math.min(...valid);
  const span = Math.max(...valid) - min || 1;
  const segments: string[] = [];
  let segment: string[] = [];
  samples.forEach((sample, index) => {
    const value = number(sample);
    if (value === null) {
      if (segment.length) segments.push(segment.join(' '));
      segment = [];
    } else {
      segment.push(`${2 + index / Math.max(1, samples.length - 1) * (width - 4)},${height - 2 - (value - min) / span * (height - 4)}`);
    }
  });
  if (segment.length) segments.push(segment.join(' '));
  return segments;
}

export function historySeries(points: EconomyHistoryPoint[]): ChartSeries[] {
  const grouped = new Map<string, EconomyHistoryPoint[]>();
  for (const point of points) {
    const group = grouped.get(point.currency) ?? [];
    group.push(point);
    grouped.set(point.currency, group);
  }
  return [...grouped].map(([currency, points]) => {
    const sorted = points.filter((point) => number(point.price) !== null && Number.isFinite(Date.parse(point.date)))
      .sort((a, b) => Date.parse(a.date) - Date.parse(b.date));
    const prices = sorted.map((point) => Number(point.price));
    const min = prices.length ? Math.min(...prices) : 0;
    const max = prices.length ? Math.max(...prices) : 0;
    const scale = priceScale(min, max);
    const start = Date.parse(sorted[0]?.date ?? '');
    const end = Date.parse(sorted.at(-1)?.date ?? '');
    const chartPoints = sorted.map((point) => ({
      x: 16 + (Date.parse(point.date) - start) / (end - start || 1) * 768,
      y: 180 - (Number(point.price) - scale.min) / (scale.max - scale.min) * 160,
      label: `${point.granularity === 'daily' ? historyDate(point.date)
        : point.date.slice(0, 16).replace('T', ' ') + ' UTC'}: ${formatNumber(point.price)} ${currency} (${point.granularity})`,
    }));
    const segments: string[] = [];
    let segment: string[] = [];
    sorted.forEach((point, index) => {
      const previous = sorted[index - 1];
      const threshold = point.granularity === 'daily' && previous?.granularity === 'daily'
        ? 36 * 60 * 60_000 : 45 * 60_000;
      if (previous && (previous.granularity !== point.granularity
        || Date.parse(point.date) - Date.parse(previous.date) > threshold)) {
        if (segment.length) segments.push(segment.join(' '));
        segment = [];
      }
      const position = chartPoints[index];
      if (position) segment.push(`${position.x},${position.y}`);
    });
    if (segment.length) segments.push(segment.join(' '));
    const dayIndices = new Map<string, number>();
    sorted.forEach((point, index) => {
      const day = historyDate(point.date);
      if (!dayIndices.has(day)) dayIndices.set(day, index);
    });
    const days = [...dayIndices];
    const tickCount = Math.min(5, days.length);
    const dayTicks: ChartTick[] = Array.from({ length: tickCount }, (_, index) => {
      const [date, pointIndex] = days[Math.round(index * (days.length - 1) / Math.max(1, tickCount - 1))]!;
      return {
        position: days.length === 1 ? 50 : chartPoints[pointIndex]!.x / 8,
        label: new Date(`${date}T00:00:00Z`).toLocaleDateString('en-CA', { month: 'short', day: 'numeric', timeZone: 'UTC' }),
      };
    });
    const volumes = sorted.map((point) => number(point.volume)).filter((volume): volume is number => volume !== null && volume >= 0);
    const volumeMax = volumes.length ? Math.max(...volumes) : 0;
    const volumeBars = sorted.flatMap((point, index) => {
      const volume = number(point.volume);
      return volume !== null && volume >= 0 ? [`M${chartPoints[index]!.x},56 V${56 - volume / (volumeMax || 1) * 52}`] : [];
    }).join(' ');
    return {
      currency, segments, points: chartPoints, min, max, priceTicks: scale.ticks, dayTicks,
      grid: [
        ...scale.ticks.map((tick) => `M16,${tick.position * 2} H784`),
        ...dayTicks.map((tick) => `M${tick.position * 8},20 V180`),
      ].join(' '),
      dots: chartPoints.map(({ x, y }) => `M${x - 2.5},${y} a2.5,2.5 0 1,0 5,0 a2.5,2.5 0 1,0 -5,0`).join(' '),
      volume: { bars: volumeBars, count: volumes.length, max: volumeMax },
    };
  });
}

export function metadataText(value: unknown): string {
  if (value === null || value === undefined) return '';
  if (typeof value === 'string') return value;
  if (Array.isArray(value)) return value.map(metadataText).filter(Boolean).join('\n');
  if (typeof value === 'object') {
    const text = Object.entries(value).find(([key]) => key === 'text' || key === 'name');
    return text ? metadataText(text[1]) : JSON.stringify(value);
  }
  return String(value);
}

import type { FlipSparklineSeries } from './api-types';
import { formatNumber } from './economy';

export interface FlipSparklineView {
  paths: string[];
  dots: { x: number; y: number }[];
  title: string;
  warnings: string[];
}

export function flipSparkline(series: FlipSparklineSeries, start: number, end: number): FlipSparklineView {
  const points = [...series.points].sort((a, b) => a.time - b.time);
  const value = (price: string | null): number | null => {
    if (price === null || !price.trim()) return null;
    const number = Number(price);
    return Number.isFinite(number) && number >= 0 ? number : null;
  };
  const valid = points.filter((point) => point.time >= start && point.time <= end
    && Number.isFinite(point.time) && value(point.price) !== null);
  const prices = valid.map((point) => value(point.price)!);
  const min = Math.min(...prices), max = Math.max(...prices);
  const paths: string[] = [], dots: { x: number; y: number }[] = [];
  let segment: { x: number; y: number }[] = [];
  const flush = () => {
    if (segment.length === 1) dots.push(segment[0]!);
    else if (segment.length > 1) paths.push(segment.map((p, i) => `${i ? 'L' : 'M'}${p.x},${p.y}`).join(' '));
    segment = [];
  };
  for (const point of points) {
    const price = value(point.price);
    if (!Number.isFinite(point.time) || point.time < start || point.time > end || price === null) { flush(); continue; }
    segment.push({
      x: end > start ? 4 + (point.time - start) / (end - start) * 112 : 60,
      y: max === min ? 18 : 32 - (price - min) / (max - min) * 28,
    });
  }
  flush();
  return { paths, dots, warnings: series.warnings, title: valid.length
    ? `Historical Chaos prices: ${valid.length} observations; sampled range ${formatNumber(String(min))}–${formatNumber(String(max))} Chaos. At most 60 evenly spaced saved hours/days; gaps at sampled observations are not interpolated. Unsampled fluctuations and gaps are not represented.`
    : 'No historical Chaos price observations available.' };
}

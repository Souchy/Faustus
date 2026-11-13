import type { GggHistory, GggMarket } from './api-types';
import { formatNumber } from './economy';

export const HOUR = 3600;
export const latestHour = (): number => Math.floor(Date.now() / 1000 / HOUR) * HOUR - HOUR;
export function localInput(hour: number): string {
  const date = new Date(hour * 1000);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}T${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`;
}
export function localDate(hour: number): string {
  return new Date(hour * 1000).toLocaleString('en-CA', { hour12: false, year: 'numeric', month: '2-digit',
    day: '2-digit', hour: '2-digit', minute: '2-digit', timeZoneName: 'short' });
}
export function parseHour(value: string): number {
  const hour = new Date(value).getTime() / 1000;
  if (!Number.isFinite(hour) || hour % HOUR !== 0) throw new Error('Choose an exact UTC hour (local time shown).');
  if (hour > latestHour()) throw new Error('The current hour and future hours are unavailable.');
  return hour;
}
export function parseRange(start: string, end: string, distinct = false): [number, number] {
  const a = parseHour(start), b = parseHour(end);
  if (b < a || (distinct && b === a)) throw new Error('End must be after start.');
  if (b - a + (distinct ? 0 : HOUR) > 180 * 24 * HOUR) throw new Error('Select at most 180 days.');
  return [a, b];
}
export function decimal(value: string, label: string): string | undefined {
  if (!value.trim()) return undefined;
  if (!/^(?:\d+(?:\.\d*)?|\.\d+)$/.test(value) || !Number.isFinite(Number(value))) {
    throw new Error(`${label} must be a nonnegative decimal.`);
  }
  return value;
}
export function meanRatio(market: GggMarket): number | null {
  const a = Number(market.volume_a), b = Number(market.volume_b);
  return a > 0 && b > 0 && Number.isFinite(b / a) ? b / a : null;
}
export function nativeRatioLabel(market: GggMarket): string {
  const ratio = meanRatio(market);
  if (ratio !== null && ratio > 0 && ratio < 1) {
    return `${formatNumber(Number(market.volume_a) / Number(market.volume_b))} ${market.a.name} per ${market.b.name}`;
  }
  return `${formatNumber(ratio)} ${market.b.name} per ${market.a.name}`;
}
export function priceSegments(history: GggHistory | null): string[] {
  if (!history) return [];
  const points = [...history.points].sort((a, b) => a.hour - b.hour);
  const valid = points.flatMap((p) => { const y = meanRatio(p.market); return y === null ? [] : [{ hour: p.hour, y }]; });
  if (!valid.length) return [];
  const min = Math.min(...valid.map((p) => p.y)), max = Math.max(...valid.map((p) => p.y));
  const span = Math.max(HOUR, history.coverage.end - history.coverage.start);
  const segments: string[] = [];
  let previous = -Infinity;
  for (const point of valid) {
    const xy = `${20 + (point.hour - history.coverage.start) / span * 560},${140 - (point.y - min) / (max - min || 1) * 120}`;
    if (point.hour - previous === HOUR) segments[segments.length - 1] += ` L${xy}`;
    else segments.push(`M${xy} h0.1`);
    previous = point.hour;
  }
  return segments;
}

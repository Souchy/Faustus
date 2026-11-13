import { HOUR, localDate } from '../../core/historical';

export function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}
export function snap(value: number, min: number, max: number, points: number[]): number {
  const bounded = clamp(Math.round(value / HOUR) * HOUR, min, max);
  const available = points.filter((point) => point >= min && point <= max);
  return available.reduce((best, point) => Math.abs(point - bounded) < Math.abs(best - bounded) ? point : best,
    available[0] ?? bounded);
}
export function observationPoints(hours: number[], days: string[]): number[] {
  return [...new Set([...hours, ...days.map((day) => Date.parse(`${day}T00:00:00Z`) / 1000)])]
    .filter(Number.isFinite).sort((a, b) => a - b);
}

export class PeriodBand {
  static get bindables(): string[] { return ['min', 'max', 'availableHours', 'availableDays', 'selectedStart', 'selectedEnd', 'commit']; }
  min = 0;
  max = 0;
  availableHours: number[] = [];
  availableDays: string[] = [];
  selectedStart = 0;
  selectedEnd = 0;
  commit: (range: { start: number; end: number }) => void = () => {};
  svg!: SVGSVGElement;
  viewStart = 0;
  viewEnd = 0;
  draftStart = 0;
  draftEnd = 0;
  private drag: { pointer: number; kind: string; x: number; start: number; end: number; viewStart: number; viewEnd: number } | null = null;
  private readonly wheelListener = (event: WheelEvent) => this.wheel(event);
  bound(): void { this.resetZoom(); this.sync(); }
  attached(): void { this.svg.addEventListener('wheel', this.wheelListener, { passive: false }); }
  detaching(): void {
    this.svg.removeEventListener('wheel', this.wheelListener);
    this.cancelDrag();
  }
  minChanged(): void { this.cancelDrag(); this.resetZoom(); }
  maxChanged(): void { this.cancelDrag(); this.resetZoom(); }
  selectedStartChanged(): void { this.sync(); }
  selectedEndChanged(): void { this.sync(); }
  private sync(): void { if (!this.drag) { this.draftStart = this.selectedStart; this.draftEnd = this.selectedEnd; } }
  get points(): number[] { return observationPoints(this.availableHours, this.availableDays); }
  get markers(): { time: number; x: number; kind: string; title: string }[] {
    const hourly = new Set(this.availableHours);
    const visible = this.points.filter((point) => point >= this.viewStart && point <= this.viewEnd);
    const stride = Math.max(1, Math.ceil(visible.length / 1000));
    return visible.filter((_, i) => i % stride === 0).map((time) => ({
      time, x: this.x(time), kind: hourly.has(time) ? 'hour' : 'day',
      title: hourly.has(time) ? `${localDate(time)} · recorded hour` : `${new Date(time * 1000).toISOString().slice(0, 10)} · UTC daily observation`,
    }));
  }
  x(time: number): number { return (time - this.viewStart) / (this.viewEnd - this.viewStart || 1) * 1000; }
  get bandX(): number { return clamp(this.x(this.draftStart), 0, 1000); }
  get bandWidth(): number { return Math.max(0, clamp(this.x(this.draftEnd), 0, 1000) - this.bandX); }
  get startX(): number { return clamp(this.x(this.draftStart), 6, 994) - 6; }
  get endX(): number { return clamp(this.x(this.draftEnd), 6, 994) - 6; }
  label = localDate;
  resetZoom(): void { this.viewStart = this.min; this.viewEnd = this.max; }
  private setView(start: number, span: number): void {
    span = clamp(span, Math.min(2 * HOUR, this.max - this.min), this.max - this.min);
    this.viewStart = clamp(start, this.min, this.max - span);
    this.viewEnd = this.viewStart + span;
  }
  wheel(event: WheelEvent): void {
    if (this.max <= this.min || !event.deltaY) return;
    const span = this.viewEnd - this.viewStart;
    if (event.shiftKey) {
      const start = this.viewStart;
      this.setView(start + event.deltaY / 600 * span, span);
      if (start === this.viewStart) return;
    } else {
      const rect = this.svg.getBoundingClientRect();
      const anchor = clamp((event.clientX - rect.left) / rect.width, 0, 1);
      const next = clamp(span * Math.exp(clamp(event.deltaY, -500, 500) / 600),
        Math.min(2 * HOUR, this.max - this.min), this.max - this.min);
      if (next === span) return;
      this.setView(this.viewStart + anchor * (span - next), next);
    }
    event.preventDefault();
  }
  pointerDown(event: PointerEvent, kind: string): void {
    if (event.button !== 0 || this.max <= this.min || this.drag) return;
    this.drag = { pointer: event.pointerId, kind, x: event.clientX, start: this.draftStart, end: this.draftEnd,
      viewStart: this.viewStart, viewEnd: this.viewEnd };
    this.svg.setPointerCapture(event.pointerId);
    event.preventDefault();
  }
  pointerMove(event: PointerEvent): void {
    const drag = this.drag;
    if (!drag || drag.pointer !== event.pointerId) return;
    const delta = (event.clientX - drag.x) / this.svg.getBoundingClientRect().width * (drag.viewEnd - drag.viewStart);
    if (drag.kind === 'pan') { this.setView(drag.viewStart - delta, drag.viewEnd - drag.viewStart); return; }
    if (drag.kind === 'start') this.draftStart = snap(drag.start + delta, this.min, this.draftEnd - HOUR, this.points);
    else if (drag.kind === 'end') this.draftEnd = snap(drag.end + delta, this.draftStart + HOUR, this.max, this.points);
    else {
      const move = clamp(Math.round(delta / HOUR) * HOUR, this.min - drag.start, this.max - drag.end);
      this.draftStart = drag.start + move; this.draftEnd = drag.end + move;
    }
  }
  pointerUp(event: PointerEvent): void {
    if (!this.drag || this.drag.pointer !== event.pointerId) return;
    const changed = this.drag.kind !== 'pan' && (this.draftStart !== this.selectedStart || this.draftEnd !== this.selectedEnd);
    this.cancelDrag();
    if (changed) this.commit({ start: this.draftStart, end: this.draftEnd });
  }
  pointerCancel(): void { if (this.drag) { this.cancelDrag(); this.sync(); } }
  private cancelDrag(): void {
    const drag = this.drag;
    this.drag = null;
    if (drag && this.svg?.hasPointerCapture(drag.pointer)) this.svg.releasePointerCapture(drag.pointer);
  }
  key(event: KeyboardEvent, kind: 'start' | 'end'): void {
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
    const min = kind === 'start' ? this.min : this.draftStart + HOUR;
    const max = kind === 'start' ? this.draftEnd - HOUR : this.max;
    const current = kind === 'start' ? this.draftStart : this.draftEnd;
    const forward = event.key === 'ArrowRight' || event.key === 'ArrowUp';
    const points = this.points.filter((point) => point >= min && point <= max);
    const next = event.key === 'Home' ? min : event.key === 'End' ? max
      : forward ? points.find((point) => point > current) ?? current + HOUR
        : points.toReversed().find((point) => point < current) ?? current - HOUR;
    if (kind === 'start') this.draftStart = snap(next, min, max, points);
    else this.draftEnd = snap(next, min, max, points);
    event.preventDefault();
    this.commit({ start: this.draftStart, end: this.draftEnd });
  }
}

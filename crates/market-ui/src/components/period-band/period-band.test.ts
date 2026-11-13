import { describe, expect, it, vi } from 'vitest';
import { observationPoints, PeriodBand, snap } from './period-band';
import { HOUR } from '../../core/historical';

function band() {
  const component = new PeriodBand();
  component.min = 0; component.max = 100 * HOUR;
  component.selectedStart = 20 * HOUR; component.selectedEnd = 60 * HOUR;
  component.availableHours = [0, 20 * HOUR, 40 * HOUR, 60 * HOUR, 100 * HOUR];
  component.svg = {
    getBoundingClientRect: () => ({ left: 0, width: 1000 }),
    setPointerCapture: vi.fn(), hasPointerCapture: () => true, releasePointerCapture: vi.fn(),
    addEventListener: vi.fn(), removeEventListener: vi.fn(),
  } as unknown as SVGSVGElement;
  component.bound(); component.commit = vi.fn();
  return component;
}
const pointer = (clientX: number) => ({ button: 0, pointerId: 1, clientX, preventDefault: vi.fn() }) as unknown as PointerEvent;

describe('recorded period band', () => {
  it('clamps and snaps reversed handle drags without requesting on each move', () => {
    const component = band();
    component.pointerDown(pointer(200), 'start'); component.pointerMove(pointer(1000));
    expect(component.draftStart).toBe(40 * HOUR);
    expect(component.draftStart).toBeLessThan(component.draftEnd); expect(component.commit).not.toHaveBeenCalled();
    component.pointerMove(pointer(-500)); expect(component.draftStart).toBe(0);
    component.pointerUp(pointer(-500)); expect(component.commit).toHaveBeenCalledTimes(1);
    expect(component.svg.releasePointerCapture).toHaveBeenCalledWith(1);
  });
  it('moves the whole band without changing duration and clamps outside bounds', () => {
    const component = band();
    component.pointerDown(pointer(400), 'band'); component.pointerMove(pointer(-1000));
    expect([component.draftStart, component.draftEnd]).toEqual([0, 40 * HOUR]);
    component.pointerMove(pointer(3000)); expect([component.draftStart, component.draftEnd]).toEqual([60 * HOUR, 100 * HOUR]);
    component.pointerUp(pointer(3000)); expect(component.commit).toHaveBeenCalledTimes(1);
  });
  it('zooms around the pointer, pans, resets, and does not consume unhandled wheel events', () => {
    const component = band();
    const event = { deltaY: -300, clientX: 250, shiftKey: false, preventDefault: vi.fn() } as unknown as WheelEvent;
    component.wheel(event);
    expect(component.viewStart + (component.viewEnd - component.viewStart) / 4).toBeCloseTo(25 * HOUR);
    expect(event.preventDefault).toHaveBeenCalledOnce();
    const before = component.viewStart; component.wheel({ ...event, deltaY: 100, shiftKey: true } as WheelEvent);
    expect(component.viewStart).toBeGreaterThan(before); expect(component.commit).not.toHaveBeenCalled();
    component.resetZoom();
    const ignored = { ...event, deltaY: 100, preventDefault: vi.fn() } as unknown as WheelEvent;
    component.wheel(ignored); expect(ignored.preventDefault).not.toHaveBeenCalled();
  });
  it('keeps only real observation ticks, bounds marker counts, and uses UTC days across DST', () => {
    expect(observationPoints([], ['2026-03-08', '2026-03-09'])).toEqual([1772928000, 1773014400]);
    const component = band(); component.availableDays = ['1970-01-02'];
    expect(component.markers.map((marker) => marker.time)).toEqual([0, 20 * HOUR, 24 * HOUR, 40 * HOUR, 60 * HOUR, 100 * HOUR]);
    component.max = 200000 * HOUR; component.resetZoom();
    component.availableHours = Array.from({ length: 200000 }, (_, i) => i * HOUR);
    expect(component.markers.length).toBeLessThanOrEqual(1000);
    expect(snap(21 * HOUR, 0, 100 * HOUR, [20 * HOUR, 40 * HOUR])).toBe(20 * HOUR);
  });
  it('supports keyboard handles and cleans up interrupted touch/pointer drags', () => {
    const component = band();
    const key = { key: 'ArrowRight', preventDefault: vi.fn() } as unknown as KeyboardEvent;
    component.key(key, 'start'); expect(component.draftStart).toBe(40 * HOUR); expect(component.commit).toHaveBeenCalledOnce();
    component.pointerDown(pointer(400), 'band'); component.pointerMove(pointer(500)); component.pointerCancel();
    expect(component.draftStart).toBe(component.selectedStart);
    component.attached(); component.detaching();
    expect(component.svg.removeEventListener).toHaveBeenCalledWith('wheel', expect.any(Function));
  });
});

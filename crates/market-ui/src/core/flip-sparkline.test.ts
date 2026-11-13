import { describe, expect, it } from 'vitest';
import { flipSparkline } from './flip-sparkline';

describe('historical Chaos sparklines', () => {
  it('positions real timestamps proportionally and breaks null gaps without endpoint interpolation', () => {
    const view = flipSparkline({ id: 'x', warnings: ['gap'], points: [
      { time: 100, price: '10' }, { time: 110, price: '20' }, { time: 120, price: null },
      { time: 180, price: '30' }, { time: 200, price: '40' },
    ] }, 100, 200);
    expect(view.paths).toEqual(['M4,32 L15.200000000000001,22.666666666666668', 'M93.60000000000001,13.333333333333336 L116,4']);
    expect(view.dots).toEqual([]); expect(view.warnings).toEqual(['gap']);
    expect(view.title).toContain('4 observations');
    expect(view.title).toContain('10–40 Chaos');
    expect(view.title).toContain('Unsampled fluctuations and gaps are not represented.');
  });
  it('centers flat series and draws isolated observations as dots, not trends', () => {
    const view = flipSparkline({ id: 'x', warnings: [], points: [
      { time: 0, price: '5' }, { time: 10, price: '5' }, { time: 20, price: null }, { time: 30, price: '5' },
    ] }, 0, 40);
    expect(view.paths).toEqual(['M4,18 L32,18']);
    expect(view.dots).toEqual([{ x: 88, y: 18 }]);
    expect(flipSparkline({ id: 'x', warnings: [], points: [{ time: 5, price: '5' }] }, 5, 5).dots).toEqual([{ x: 60, y: 18 }]);
  });
  it('does not turn absent, invalid or out-of-period prices into zero observations', () => {
    const view = flipSparkline({ id: 'x', warnings: [], points: [
      { time: -1, price: '5' }, { time: 0, price: null }, { time: 1, price: '' },
      { time: 2, price: 'bad' }, { time: 3, price: '-1' },
    ] }, 0, 4);
    expect(view.paths).toEqual([]); expect(view.dots).toEqual([]);
    expect(view.title).toBe('No historical Chaos price observations available.');
    expect(flipSparkline({ id: 'x', warnings: [], points: [{ time: 0, price: '0' }] }, 0, 1).dots).toHaveLength(1);
  });
});

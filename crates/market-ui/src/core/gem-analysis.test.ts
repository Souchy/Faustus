import { describe, expect, it } from 'vitest';
import {
  decimal, exchangePrice, filterAndSort, gemType, opportunities,
  type GemAnalysisInputs,
} from './gem-analysis';
import { categorizedGemOverview, gem, gemOverview, prices } from '../testing/fixtures';

function inputs(): GemAnalysisInputs {
  return { gcpPrice: 1, vaalPrice: 2, lensPrice: null, useLens: false, confidenceThreshold: 5 };
}

describe('same-instant gem analysis', () => {
  it('uses provider categories before ordinary skill/support names without guessing from "of"', () => {
    expect(gemType('Fireball', null)).toBe('Skill');
    expect(gemType('Multistrike Support', undefined)).toBe('Support');
    expect(gemType('Empower Support', 'exceptional')).toBe('Exceptional');
    expect(gemType('Cleave of Rage', 'transfigured')).toBe('Transfigured');
    expect(gemType('Sunder of Trarthus', 'trarthus')).toBe('Trarthus');
    expect(gemType('Purity of Fire', null)).toBe('Skill');
    expect(gemType('Herald of Ash', null)).toBe('Skill');
    expect(gemType('Future Support', 'future')).toBe('Unknown');
    expect(gemType('Future Gem', 'support')).toBe('Support');
  });

  it('carries one category through every strategy and uses metadata from any variant', () => {
    const overview = categorizedGemOverview();
    const expected = ['Transfigured', 'Exceptional', 'Skill', 'Support', 'Trarthus'];
    for (const strategy of ['level', 'quality', 'corrupt-level', 'corrupt-quality'] as const) {
      expect(opportunities(overview.gems, strategy, inputs()).map((row) => row.gemType)).toEqual(expected);
    }
    const gems = [
      { ...gem(1, 0, false, '2'), name: 'Empower Support' },
      { ...gem(3, 0, false, '40'), name: 'Empower Support', gem_kind: 'exceptional' },
    ];
    expect(opportunities(gems, 'level', inputs())[0]?.gemType).toBe('Exceptional');
  });

  it('combines type, search and confidence filters before the result cap and sorts by type', () => {
    const rows = opportunities(categorizedGemOverview().gems, 'level', inputs());
    const skill = rows.find((row) => row.gemType === 'Skill');
    const support = rows.find((row) => row.gemType === 'Support');
    if (!skill || !support) throw new Error('Expected categorized fixture opportunities');
    const many = [...Array.from({ length: 125 }, () => ({ ...skill, profit: 100 })), support];
    expect(filterAndSort(many, '', false, 'profit', false, ['Support'])).toEqual([support]);
    expect(filterAndSort(rows, 'EMPOWER', false, 'profit', false, ['Exceptional', 'Support'])).toHaveLength(1);
    expect(filterAndSort(rows, 'Fireball', false, 'profit', false, ['Support', 'Exceptional'])).toEqual([]);
    expect(filterAndSort([{ ...support, lowConfidence: true }], '', false, 'profit', false, ['Support'])).toEqual([]);
    expect(filterAndSort([{ ...support, lowConfidence: true }], '', true, 'profit', false, ['Support'])).toHaveLength(1);
    expect(filterAndSort(many, '', false, 'profit', false, ['Support', 'Exceptional'])).toEqual([support]);
    expect(filterAndSort(rows, '', false, 'profit', false, ['Skill', 'Transfigured']).map((row) => row.gemType).sort())
      .toEqual(['Skill', 'Transfigured']);
    expect(filterAndSort(rows, '', false, 'profit', false, [])).toEqual([]);
    const unknown = { ...support, gemType: 'Unknown' as const };
    expect(filterAndSort([...rows, unknown], '', false, 'profit', false, ['Unknown'])).toEqual([unknown]);
    expect(filterAndSort(rows, '', false, 'gemType', true).map((row) => row.gemType))
      .toEqual(['Exceptional', 'Skill', 'Support', 'Transfigured', 'Trarthus']);
    expect(filterAndSort(rows, '', false, 'gemType', false)[0]?.gemType).toBe('Trarthus');
  });

  it('matches all four strategies and subtracts material costs', () => {
    const gems = gemOverview().gems;
    expect(opportunities(gems, 'level', inputs())[0]?.profit).toBe(38);
    const lens = opportunities(gems, 'level', { ...inputs(), lensPrice: 5, useLens: true })[0];
    expect(lens?.materialCost).toBe(5);
    expect(lens?.profit).toBe(33);
    const quality = opportunities(gems, 'quality', inputs())[0];
    expect(quality?.materialCost).toBe(20);
    expect(quality?.profit).toBe(40);
    expect(opportunities(gems, 'corrupt-level', inputs())[0]?.profit).toBe(38);
    expect(opportunities(gems, 'corrupt-quality', inputs())[0]?.profit).toBe(18);
  });

  it('uses the selected snapshot prices, not different sparkline endpoints', () => {
    const gems = gemOverview().gems.map((gem) => ({
      ...gem, sparkline: ['-99', null, '300'], sparkline_change: '300',
    }));
    expect(opportunities(gems, 'level', inputs())[0]).toMatchObject({ sourcePrice: 2, targetPrice: 40, profit: 38 });
    const overview = prices();
    overview.lines = overview.lines.map((line) => ({ ...line, sparkline: [], sparkline_change: '-100' }));
    expect(exchangePrice(overview, 'gcp')).toBe(1);
  });

  it('rejects missing, invalid, negative and non-finite listed prices', () => {
    for (const value of ['', ' ', 'NaN', 'Infinity', 'invalid']) expect(decimal(value)).toBeNull();
    for (const value of [null, '-1', 'Infinity', 'NaN', 'invalid', '']) {
      for (const index of [0, 1]) {
        const gems = gemOverview().gems.map((gem, i) => i === index ? { ...gem, chaos_value: value } : gem);
        expect(opportunities(gems, 'level', inputs())).toEqual([]);
      }
      const overview = prices();
      const line = overview.lines[0];
      if (!line) throw new Error('Expected material fixture');
      line.primary_value = value;
      expect(exchangePrice(overview, 'gcp')).toBeNull();
    }
  });

  it('requires material prices and valid lens input only for the relevant strategies', () => {
    const gems = gemOverview().gems;
    expect(opportunities(gems, 'quality', { ...inputs(), gcpPrice: null })).toEqual([]);
    expect(opportunities(gems, 'corrupt-level', { ...inputs(), vaalPrice: null })).toEqual([]);
    for (const lensPrice of [null, -1, 0, Infinity, NaN]) {
      expect(opportunities(gems, 'level', { ...inputs(), lensPrice, useLens: true })).toEqual([]);
    }
    expect(opportunities(gems, 'level', { ...inputs(), gcpPrice: null, vaalPrice: null })).toHaveLength(1);
    expect(exchangePrice(prices(), 'gcp')).toBe(1);
    expect(exchangePrice(prices(), 'missing')).toBeNull();
  });

  it('applies the configurable minimum to samples and listings on both variants, including exact boundaries', () => {
    const variants = [gem(1, 0, false, '2'), gem(20, 0, false, '40')];
    for (const index of [0, 1]) {
      for (const field of ['sample_count', 'listing_count'] as const) {
        for (const [count, low] of [[4, true], [5, false], [6, false], [null, true]] as const) {
          const gems = variants.map((gem, i) => i === index ? { ...gem, [field]: count } : gem);
          expect(opportunities(gems, 'level', inputs())[0]?.lowConfidence).toBe(low);
        }
      }
    }
    expect(opportunities(variants, 'level', { ...inputs(), confidenceThreshold: 12 })[0]?.lowConfidence).toBe(false);
    expect(opportunities(variants, 'level', { ...inputs(), confidenceThreshold: 13 })[0]?.lowConfidence).toBe(true);
  });
  it('handles missing variants and low confidence, and filters before the 100-row cap', () => {
    const source = { ...gem(1, 0, false, '2'), sample_count: 2, listing_count: null };
    const low = opportunities([source, gem(20, 0, false, '40')], 'level', inputs());
    expect(low[0]?.lowConfidence).toBe(true);
    expect(low[0]?.sourceListings).toBe(0);
    expect(filterAndSort(low, ' FIRE ', true, 'profit', false)).toHaveLength(1);
    expect(filterAndSort(low, 'fire', false, 'profit', false)).toEqual([]);
    expect(opportunities([source], 'level', inputs())).toEqual([]);
    const row = opportunities(gemOverview().gems, 'level', inputs())[0];
    if (!row) throw new Error('Expected fixture opportunity');
    const rows = Array.from({ length: 125 }, (_, i) => ({ ...row, name: `Gem ${i}`, profit: i }));
    expect(filterAndSort(rows, '', true, 'profit', false)).toHaveLength(100);
    expect(filterAndSort(rows, '', true, 'profit', false)[0]?.profit).toBe(124);
    expect(filterAndSort(rows, 'Gem 0', true, 'profit', false)).toHaveLength(1);
    expect(filterAndSort(rows, '', true, 'profit', true)[0]?.profit).toBe(0);
  });

  it('sorts text, numeric and combined listing columns in both directions', () => {
    const row = opportunities(gemOverview().gems, 'level', inputs())[0];
    if (!row) throw new Error('Expected fixture opportunity');
    const rows = [{ ...row, name: 'Fireball', sourceListings: 1, targetListings: 2 },
      { ...row, name: 'Arc', sourceListings: 8, targetListings: 12 }];
    expect(filterAndSort(rows, '', true, 'name', true)[0]?.name).toBe('Arc');
    expect(filterAndSort(rows, '', true, 'name', false)[0]?.name).toBe('Fireball');
    expect(filterAndSort(rows, '', true, 'listings', false)[0]?.name).toBe('Arc');
  });
});

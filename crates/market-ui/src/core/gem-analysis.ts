import type { CurrentExchangePriceOverview, GemMarketLine } from './api-types';

export type GemStrategy = 'level' | 'quality' | 'corrupt';
export type GemSortColumn = 'name' | 'gemType' | 'sourceVariant' | 'sourcePrice' | 'targetVariant'
  | 'targetPrice' | 'maxValue' | 'materialCost' | 'profit' | 'roiPercent' | 'listings';
export type GemType = 'Skill' | 'Support' | 'Exceptional' | 'Transfigured' | 'Trarthus' | 'Unknown';
export const gemTypes: GemType[] = ['Skill', 'Support', 'Exceptional', 'Transfigured', 'Trarthus', 'Unknown'];

export function gemType(name: string, kind: string | null | undefined): GemType {
  switch (kind) {
    case 'skill': return 'Skill';
    case 'support': return 'Support';
    case 'exceptional': return 'Exceptional';
    case 'transfigured': return 'Transfigured';
    case 'trarthus': return 'Trarthus';
    default: return kind ? 'Unknown' : name.endsWith(' Support') ? 'Support' : 'Skill';
  }
}

export const strategies: { id: GemStrategy; label: string; description: string }[] = [
  { id: 'level', label: 'Level flip', description: 'Buy level 1, sell the highest uncorrupted level. Leveling time excluded; Lens is optional.' },
  { id: 'quality', label: 'Add quality', description: "Compare max-level 0% and 20% quality; cost includes 20 Gemcutter's Prisms." },
  { id: 'corrupt', label: 'Vaal corrupt', description: 'Approximate expected returns across four corruption outcome buckets.' },
];

export function decimal(value: string | null | undefined): number | null {
  if (value == null || value.trim() === '') return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function listedPrice(value: string | null | undefined): number | null {
  const price = decimal(value);
  return price !== null && price >= 0 ? price : null;
}

export function exchangePrice(
  overview: CurrentExchangePriceOverview | null, id: string,
): number | null {
  const line = overview?.lines.find((entry) => entry.id === id);
  return listedPrice(line?.primary_value);
}

export interface GemAnalysisInputs {
  historical?: boolean;
  gcpPrice: number | null;
  vaalPrice: number | null;
  lensPrice: number | null;
  useLens: boolean;
  confidenceThreshold: number;
}

export interface GemOpportunity {
  name: string;
  gemType: GemType;
  sourceVariant: string;
  targetVariant: string;
  sourcePrice: number;
  targetPrice: number;
  maxValue: number;
  materialCost: number;
  profit: number;
  roiPercent: number;
  sourceListings: number | null;
  targetListings: number | null;
  unknownConfidence?: boolean;
  lowConfidence: boolean;
  corruptionOutcomes?: CorruptionOutcome[];
}

export interface CorruptionOutcome {
  label: string;
  probability: number;
  price: number;
  contribution: number;
  variant: string;
  listings: number | null;
  estimated: boolean;
}

function variantLabel(gem: GemMarketLine): string {
  return `Lv${gem.gem_level ?? 0} Q${gem.gem_quality ?? 0}${gem.corrupted ? ' corrupted' : ''}`;
}

function lowConfidence(gem: GemMarketLine, threshold: number): boolean {
  return (gem.sample_count ?? 0) < threshold || (gem.listing_count ?? 0) < threshold;
}

export function opportunities(
  gems: GemMarketLine[], strategy: GemStrategy, inputs: GemAnalysisInputs,
): GemOpportunity[] {
  if (strategy === 'level' && inputs.useLens
    && (inputs.lensPrice === null || !Number.isFinite(inputs.lensPrice) || inputs.lensPrice <= 0)) return [];
  if (strategy === 'quality' && inputs.gcpPrice === null) return [];
  if (strategy === 'corrupt' && inputs.vaalPrice === null) return [];

  const groups = new Map<string, GemMarketLine[]>();
  for (const gem of gems) {
    const group = groups.get(gem.name);
    if (group) group.push(gem);
    else groups.set(gem.name, [gem]);
  }
  const rows: GemOpportunity[] = [];
  // Match the old BTreeMap iteration order for stable sorting of equal values.
  for (const [name, variants] of [...groups].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)) {
    const find = (level: number, quality: number, corrupted: boolean) => variants.find(
      (gem) => gem.gem_level === level && (gem.gem_quality ?? 0) === quality && gem.corrupted === corrupted,
    );
    const maxLevel = (quality: number | null) => Math.max(0, ...variants
      .filter((gem) => !gem.corrupted && (quality === null || (gem.gem_quality ?? 0) === quality))
      .map((gem) => gem.gem_level ?? 0));
    let source: GemMarketLine | undefined;
    let target: GemMarketLine | undefined;
    let materialCost = 0;
    let corruptionOutcomes: CorruptionOutcome[] | undefined;
    let outcomeVariants: GemMarketLine[] = [];
    if (strategy === 'level') {
      const level = maxLevel(0);
      if (level <= 1) continue;
      source = find(1, 0, false);
      target = find(level, 0, false);
      materialCost = inputs.useLens ? inputs.lensPrice ?? 0 : 0;
    } else if (strategy === 'quality') {
      const level = maxLevel(null);
      source = find(level, 0, false);
      target = find(level, 20, false);
      materialCost = (inputs.gcpPrice ?? 0) * 20;
    } else {
      const level = maxLevel(20);
      source = find(level, 20, false);
      materialCost = inputs.vaalPrice ?? 0;
      const buckets = [
        { label: '+1 level', probability: 0.125, level: level + 1, qualities: [20], preferred: 20 },
        { label: '+3 quality', probability: 0.125, level, qualities: [21, 22, 23], preferred: 23 },
        { label: 'Max level corrupted (including releveled -1)', probability: 0.625, level, qualities: [20], preferred: 20 },
        { label: 'Max level Q0-19 corrupted', probability: 0.125, level,
          qualities: Array.from({ length: 20 }, (_, quality) => quality), preferred: null },
      ];
      corruptionOutcomes = [];
      for (const bucket of buckets) {
        const candidates = variants.filter((gem) => gem.corrupted && gem.gem_level === bucket.level
          && bucket.qualities.includes(gem.gem_quality ?? 0) && listedPrice(gem.chaos_value) !== null);
        const preferred = candidates.find((gem) => gem.gem_quality === bucket.preferred);
        const selected = preferred ?? candidates.sort((a, b) => Number(a.chaos_value) - Number(b.chaos_value))[0];
        if (!selected) break;
        const price = listedPrice(selected.chaos_value);
        if (price === null) break;
        outcomeVariants.push(selected);
        corruptionOutcomes.push({
          label: bucket.label, probability: bucket.probability, price,
          contribution: bucket.probability * price, variant: variantLabel(selected),
          listings: selected.listing_count ?? null,
          estimated: !preferred,
        });
      }
      if (corruptionOutcomes.length !== buckets.length) continue;
      target = outcomeVariants[0];
    }
    if (!source || !target) continue;
    const sourcePrice = listedPrice(source.chaos_value);
    const targetPrice = corruptionOutcomes
      ? corruptionOutcomes.reduce((sum, outcome) => sum + outcome.contribution, 0)
      : listedPrice(target.chaos_value);
    if (sourcePrice === null || targetPrice === null) continue;
    const totalCost = sourcePrice + materialCost;
    if (totalCost <= 0) continue;
    rows.push({
      name, gemType: gemType(name, variants.find((gem) => gem.gem_kind)?.gem_kind),
      sourceVariant: variantLabel(source), targetVariant: corruptionOutcomes ? 'Expected resale (approx.)' : variantLabel(target),
      sourcePrice, targetPrice, materialCost, profit: targetPrice - totalCost,
      maxValue: corruptionOutcomes ? Math.max(...corruptionOutcomes.map((outcome) => outcome.price)) : targetPrice,
      roiPercent: (targetPrice / totalCost - 1) * 100,
      sourceListings: inputs.historical ? null : source.listing_count ?? 0,
      targetListings: inputs.historical ? null : corruptionOutcomes
        ? Math.min(...outcomeVariants.map((gem) => gem.listing_count ?? 0)) : target.listing_count ?? 0,
      unknownConfidence: inputs.historical === true,
      lowConfidence: !inputs.historical && (lowConfidence(source, inputs.confidenceThreshold)
        || (corruptionOutcomes ? outcomeVariants : [target]).some((gem) => lowConfidence(gem, inputs.confidenceThreshold))),
      corruptionOutcomes,
    });
  }
  return rows;
}

export function filterAndSort(
  rows: GemOpportunity[], search: string, includeLowConfidence: boolean,
  column: GemSortColumn, ascending: boolean, types: readonly GemType[] = gemTypes,
): GemOpportunity[] {
  const query = search.trim().toLowerCase();
  return rows.filter((row) => row.name.toLowerCase().includes(query)
    && (includeLowConfidence || !row.lowConfidence) && types.includes(row.gemType))
    .sort((a, b) => {
      const left = column === 'listings' ? (a.sourceListings ?? 0) + (a.targetListings ?? 0) : a[column];
      const right = column === 'listings' ? (b.sourceListings ?? 0) + (b.targetListings ?? 0) : b[column];
      const order = left < right ? -1 : left > right ? 1 : 0;
      return ascending ? order : -order;
    }).slice(0, 100);
}

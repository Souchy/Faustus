import { CustomElement, INode, resolve } from 'aurelia';
import type { EconomyArchive, EconomyLeague } from '../../core/api-types';
import { formatNumber } from '../../core/economy';
import { MarketApi } from '../../core/market-api';
import template from './dump-manager.html?raw';

export class DumpManager {
  private readonly api = resolve(MarketApi);
  private readonly host = resolve(INode);
  private archiveRequest: AbortController | null = null;
  private importRequest: AbortController | null = null;
  private poll: ReturnType<typeof setTimeout> | null = null;
  private active = false;
  leagues: EconomyLeague[] = [];
  archives: EconomyArchive[] = [];
  archiveLoading = false;
  archiveError = '';
  importing = '';
  notice = '';
  ninjaErrors: Record<string, string> = {};
  gggErrors: Record<string, string> = {};
  readonly seeds = ['Ancestors', 'Mirage', 'Phrecia 2.0', 'Keepers', 'Mercenaries'];
  format = formatNumber;

  attached(): void {
    this.active = true;
    void this.loadArchives();
  }

  unbinding(): void {
    this.active = false;
    this.archiveRequest?.abort();
    this.archiveRequest = null;
    this.importRequest?.abort();
    this.importRequest = null;
    if (this.poll) clearTimeout(this.poll);
    this.poll = null;
  }

  async loadArchives(refresh = false): Promise<void> {
    if (!this.active) return;
    this.archiveRequest?.abort();
    if (this.poll) clearTimeout(this.poll);
    this.poll = null;
    const request = new AbortController();
    this.archiveRequest = request;
    this.archiveLoading = true;
    this.archiveError = '';
    try {
      const archives = await this.api.economyArchives(request.signal, refresh);
      if (this.archiveRequest !== request || !this.active) return;
      this.archives = archives;
      const leagues = await this.api.economyLeagues(request.signal);
      if (this.archiveRequest !== request || !this.active) return;
      this.leagues = leagues;
      if (archives.some((archive) => ['queued', 'downloading', 'importing'].includes(archive.status)
        || archive.ggg?.status === 'running')) {
        this.poll = setTimeout(() => { void this.loadArchives(); }, 3000);
      }
    } catch (error: unknown) {
      if (this.archiveRequest === request && !request.signal.aborted) {
        this.archiveError = `Unable to load archive status: ${this.message(error)}`;
      }
    } finally {
      if (this.archiveRequest === request) this.archiveLoading = false;
    }
  }

  async importArchive(name: string, source: 'both' | 'ninja' | 'ggg' = 'both'): Promise<void> {
    if (!this.active) return;
    this.importing = name;
    this.archiveError = this.notice = '';
    const request = new AbortController();
    this.importRequest = request;
    try {
      const sources = source === 'both' ? ['ninja', 'ggg'] as const : [source];
      const results = await Promise.allSettled(sources.map((source) => source === 'ninja'
        ? this.api.importEconomyArchive(name, request.signal)
        : this.api.importGggArchive(name, request.signal)));
      if (this.importRequest !== request || !this.active) return;
      const requested: string[] = [];
      sources.forEach((source, index) => {
        const result = results[index]!;
        const errors = source === 'ninja' ? this.ninjaErrors : this.gggErrors;
        const label = source === 'ninja' ? 'poe.ninja' : 'GGG PC';
        errors[name] = result.status === 'rejected' ? `Unable to start ${label} import: ${this.message(result.reason)}` : '';
        if (result.status === 'fulfilled') requested.push(label);
      });
      this.notice = requested.length ? `Import requested for ${name}: ${requested.join(' and ')}. Sources run independently in the background.` : '';
      await this.loadArchives();
    } catch (error: unknown) {
      if (this.importRequest === request && !request.signal.aborted) {
        this.archiveError = `Unable to import ${name}: ${this.message(error)}`;
      }
    } finally {
      if (this.importRequest === request) {
        this.importing = '';
        this.importRequest = null;
      }
    }
  }

  browseArchive(archive: EconomyArchive): void {
    this.host.dispatchEvent(new CustomEvent<EconomyArchive>('browse-archive', { detail: archive, bubbles: true }));
  }

  formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    const units = ['KB', 'MB', 'GB'];
    let value = bytes / 1024;
    let unit = units[0]!;
    for (let index = 1; value >= 1024 && index < units.length; index++) {
      value /= 1024;
      unit = units[index]!;
    }
    return `${value.toFixed(1)} ${unit}`;
  }

  downloadProgress(archive: EconomyArchive): string {
    const total = archive.download_total_bytes;
    if (!total) return `Downloaded ${this.formatBytes(archive.downloaded_bytes)} · total size unavailable.`;
    const percent = Math.floor(100 * archive.downloaded_bytes / total);
    return `Downloaded ${this.formatBytes(archive.downloaded_bytes)} of ${this.formatBytes(total)} (${percent}%).`;
  }

  importProgress(archive: EconomyArchive): string {
    const total = archive.total_rows;
    if (total == null) return 'CSV row total unavailable.';
    const processed = archive.rows + archive.skipped;
    const percent = total > 0 ? Math.floor(100 * processed / total) : 0;
    return `${this.format(processed)} / ${this.format(total)} CSV rows processed (${percent}%). Includes excluded rows.`;
  }

  hourPercent(archive: EconomyArchive): number {
    return archive.ggg?.total_hours ? Math.floor(100 * archive.ggg.completed_hours / archive.ggg.total_hours) : 0;
  }

  utcHour(hour: number): string {
    return `${new Date(hour * 1000).toISOString().slice(0, 16).replace('T', ' ')} UTC`;
  }

  get allflameAvailable(): boolean { return this.archives.some((archive) => archive.name === 'Allflame'); }
  private message(error: unknown): string { return error instanceof Error ? error.message : String(error); }
}

CustomElement.define({ name: 'dump-manager', template, bindables: ['leagues'] }, DumpManager);

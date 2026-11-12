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
      if (archives.some((archive) => ['queued', 'downloading', 'importing'].includes(archive.status))) {
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

  async importArchive(name: string): Promise<void> {
    if (!this.active) return;
    this.importing = name;
    this.archiveError = this.notice = '';
    const request = new AbortController();
    this.importRequest = request;
    try {
      await this.api.importEconomyArchive(name, request.signal);
      if (this.importRequest !== request || !this.active) return;
      this.notice = `Import requested for ${name}. Only softcore challenge data will be imported.`;
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

  get allflameAvailable(): boolean { return this.archives.some((archive) => archive.name === 'Allflame'); }
  private message(error: unknown): string { return error instanceof Error ? error.message : String(error); }
}

CustomElement.define({ name: 'dump-manager', template, bindables: ['leagues'] }, DumpManager);

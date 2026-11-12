import { resolve } from 'aurelia';
import { MarketSession } from '../../services/market-session';
import type { DumpManager } from './dump-manager';

export class SettingsPage {
  readonly session = resolve(MarketSession);
  dumpManager: DumpManager | null = null;

  get isLoading(): boolean { return this.dumpManager?.archiveLoading ?? false; }

  async refresh(): Promise<void> {
    await this.dumpManager?.loadArchives(true);
  }

  loading(): void {
    this.session.page = 'settings';
    this.session.settings = this;
  }

  unbinding(): void {
    if (this.session.settings === this) this.session.settings = null;
  }
}

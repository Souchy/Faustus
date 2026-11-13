import Aurelia, { Registration } from 'aurelia';
import { RouterConfiguration } from '@aurelia/router';
import { MarketApi } from './core/market-api';
import { MarketSession } from './services/market-session';
import { MarketApp } from './market-app';
import './styles.css';

async function start(): Promise<void> {
  await new Aurelia()
    .register(
      RouterConfiguration.customize({ useUrlFragmentHash: true, basePath: '/' }),
      Registration.singleton(MarketApi, MarketApi),
      Registration.singleton(MarketSession, MarketSession),
    )
    .app(MarketApp)
    .start();
}

void start().catch((error: unknown) => {
  console.error('Unable to start Faust', error);
  const host = document.querySelector('market-app');
  if (host) {
    host.textContent = `Unable to start Faust: ${error instanceof Error ? error.message : String(error)}`;
    host.setAttribute('role', 'alert');
  }
});

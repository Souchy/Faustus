import Aurelia, { Registration } from 'aurelia';
import { MarketApi } from './core/market-api';
import { MarketApp } from './market-app';
import './styles.css';

async function start(): Promise<void> {
  await new Aurelia()
    .register(Registration.singleton(MarketApi, MarketApi))
    .app(MarketApp)
    .start();
}

void start().catch((error: unknown) => {
  console.error('Unable to start Faustus', error);
  const host = document.querySelector('market-app');
  if (host) {
    host.textContent = `Unable to start Faustus: ${error instanceof Error ? error.message : String(error)}`;
    host.setAttribute('role', 'alert');
  }
});

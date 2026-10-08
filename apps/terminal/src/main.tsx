import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import './index.css';
import { App } from './App';
import { AuthGate } from './components/Auth';
import { ChartWindow } from './components/ChartWindow';
import { parseChartView, prepareChartWindow, startNativeBridge } from './native';
import { createApiFromLocation, isGatewayApi } from './store/api';
import { loadGateway } from './store/connection';
import { identityUrl, useSession } from './store/session';
import { registerServiceWorker } from './lib/install';
import { captureReferral } from './lib/referral';
import { loadBrand } from './lib/brand';

const chartView = parseChartView(window.location.search);
if (chartView) prepareChartWindow(chartView);

const api = createApiFromLocation(window.location.search);
// A gateway without a pasted dev token signs in through services/identity
// (VITE_IDENTITY_URL). The mock simulator never needs a login.
const idUrl = identityUrl();
const gated = !!idUrl && isGatewayApi(api) && !loadGateway()?.token;
if (gated) useSession.getState().configure(idUrl);
// Desktop shell only: tray status + OS notifications (the main window owns these).
if (!chartView) startNativeBridge(api);
registerServiceWorker();
captureReferral();
void loadBrand();

const body = chartView ? <ChartWindow api={api} view={chartView} /> : <App api={api} />;

createRoot(document.getElementById('root')!).render(
  <StrictMode>{gated ? <AuthGate>{body}</AuthGate> : body}</StrictMode>,
);

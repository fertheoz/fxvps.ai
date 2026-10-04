import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import './index.css';
import { App } from './App';
import { AuthGate } from './components/Auth';
import { createApiFromLocation, isGatewayApi } from './store/api';
import { loadGateway } from './store/connection';
import { identityUrl, useSession } from './store/session';

const api = createApiFromLocation(window.location.search);
// A gateway without a pasted dev token signs in through services/identity
// (VITE_IDENTITY_URL). The mock simulator never needs a login.
const idUrl = identityUrl();
const gated = !!idUrl && isGatewayApi(api) && !loadGateway()?.token;
if (gated) useSession.getState().configure(idUrl);

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {gated ? (
      <AuthGate>
        <App api={api} />
      </AuthGate>
    ) : (
      <App api={api} />
    )}
  </StrictMode>,
);

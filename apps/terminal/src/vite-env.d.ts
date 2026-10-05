/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Base URL of services/identity (e.g. https://id.fxvps.ai). Empty: no login screen. */
  readonly VITE_IDENTITY_URL?: string;
  /**
   * Comma separated gateway origins (`wss://gw.fxvps.ai`) the terminal may connect to
   * with the identity session token. The page origin and (dev / loopback pages)
   * loopback gateways are always allowed.
   */
  readonly VITE_ALLOWED_WS_ORIGINS?: string;
  /** Gateway used when none is configured (`/ws` = same host). */
  readonly VITE_DEFAULT_WS_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Base URL of services/identity (e.g. https://id.fxvps.ai). Empty: no login screen. */
  readonly VITE_IDENTITY_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

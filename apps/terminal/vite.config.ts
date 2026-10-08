/// <reference types="vitest/config" />
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import type { Plugin } from 'vite';

// G13: page CSP for production builds only (the dev server injects an inline
// React refresh preamble). The gateway/identity URL is user-configurable, so
// connect-src allows any https/wss origin plus loopback http/ws for local e2e.
const CSP = [
  "default-src 'self'",
  "script-src 'self'",
  "style-src 'self' 'unsafe-inline'",
  // https: white-label tenant logos (lib/brand.ts)
  "img-src 'self' data: blob: https:",
  "font-src 'self' data:",
  "connect-src 'self' https: wss: http://localhost:* http://127.0.0.1:* ws://localhost:* ws://127.0.0.1:*",
  "worker-src 'self' blob:",
  "object-src 'none'",
  "base-uri 'self'",
  "form-action 'self'",
].join('; ');

const cspMeta = (): Plugin => ({
  name: 'fxvps-csp',
  apply: 'build',
  transformIndexHtml: () => [
    { tag: 'meta', attrs: { 'http-equiv': 'Content-Security-Policy', content: CSP }, injectTo: 'head-prepend' },
  ],
});

export default defineConfig({
  plugins: [react(), tailwindcss(), cspMeta()],
  resolve: {
    // Shared pure-TS package without its own node_modules: its bare imports
    // (big.js) resolve from this app via dedupe.
    alias: {
      '@fxvps/trading-core': fileURLToPath(new URL('../../packages/trading-core/src/index.ts', import.meta.url)),
    },
    dedupe: ['big.js'],
  },
  build: {
    target: 'es2022',
    chunkSizeWarningLimit: 900,
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}', '../../packages/trading-core/src/**/*.test.ts'],
  },
});

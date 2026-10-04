import { spawn, type ChildProcess } from 'node:child_process';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const START_TIMEOUT_MS = Number(process.env.FXVPS_START_TIMEOUT_MS ?? 900_000);
const IDENTITY = 'http://localhost:18090';
const ORIGIN = 'http://localhost:4175';
/** Test-only service token (admin API); the identity process exists only for this run. */
const SERVICE_TOKEN = 'e2e-service-token-not-a-secret-0123456789';

function start(name: string, pkg: string, bin: string | undefined, args: string[], env: Record<string, string>, pattern: RegExp): Promise<{ child: ChildProcess; value: string }> {
  const fullEnv = { ...process.env, RUST_LOG: process.env.RUST_LOG ?? 'warn', ...env };
  const child = bin
    ? spawn(bin, args, { cwd: repoRoot, stdio: ['ignore', 'pipe', 'inherit'], env: fullEnv })
    : spawn('cargo', ['run', '--quiet', '-p', pkg, '--', ...args], { cwd: repoRoot, stdio: ['ignore', 'pipe', 'inherit'], env: fullEnv });
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`${name} did not start in time`)), START_TIMEOUT_MS);
    child.once('exit', (code) => {
      clearTimeout(timer);
      reject(new Error(`${name} exited early (code ${code})`));
    });
    createInterface({ input: child.stdout! }).on('line', (line) => {
      // eslint-disable-next-line no-control-regex
      const m = pattern.exec(line.replace(/\x1b\[[0-9;]*m/g, ''));
      if (m) {
        clearTimeout(timer);
        resolve({ child, value: m[1]! });
      }
    });
  });
}

async function stop(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null) return;
  child.kill('SIGINT');
  await new Promise<void>((r) => {
    const t = setTimeout(() => {
      child.kill('SIGKILL');
      r();
    }, 5_000);
    child.once('exit', () => {
      clearTimeout(t);
      r();
    });
  });
}

/** Starts identity + client-gateway (JWKS URL) when FXVPS_IDENTITY_E2E_LIVE=1. */
export default async function globalSetup(): Promise<() => Promise<void>> {
  if (process.env.FXVPS_IDENTITY_E2E_LIVE !== '1') return async () => undefined;
  const mailFile = path.join(mkdtempSync(path.join(tmpdir(), 'fxvps-id-')), 'mail.jsonl');
  const id = await start(
    'identity',
    'identity',
    process.env.FXVPS_IDENTITY_BIN,
    [],
    {
      IDENTITY_LISTEN: '127.0.0.1:18090',
      IDENTITY_ISSUER: IDENTITY,
      IDENTITY_RP_ID: 'localhost',
      IDENTITY_RP_ORIGIN: ORIGIN,
      IDENTITY_ALLOWED_ORIGINS: ORIGIN,
      IDENTITY_SERVICE_TOKEN: SERVICE_TOKEN,
      IDENTITY_DEV_MAIL_FILE: mailFile,
      IDENTITY_IP_REQUESTS_PER_MINUTE: '1000',
    },
    /FXVPS_IDENTITY_URL=(\S+)/,
  );
  let gw: { child: ChildProcess; value: string };
  try {
    gw = await start(
      'client-gateway',
      'client-gateway',
      process.env.FXVPS_GATEWAY_BIN,
      ['--demo', '--listen', '127.0.0.1:0'],
      {
        FXVPS_JWT_JWKS_URL: 'http://127.0.0.1:18090/.well-known/jwks.json',
        FXVPS_JWT_ISSUER: IDENTITY,
        FXVPS_JWT_AUDIENCE: 'fxvps',
      },
      /FXVPS_WS_URL=(\S+)/,
    );
  } catch (e) {
    await stop(id.child);
    throw e;
  }
  process.env.FXVPS_WS_URL = gw.value;
  process.env.FXVPS_IDENTITY_MAIL_FILE = mailFile;
  process.env.FXVPS_IDENTITY_SERVICE_TOKEN = SERVICE_TOKEN;
  process.env.FXVPS_IDENTITY_LIVE_READY = '1';
  console.log(`identity at ${id.value}, client-gateway at ${gw.value}`);
  return async () => {
    await stop(gw.child);
    await stop(id.child);
  };
}

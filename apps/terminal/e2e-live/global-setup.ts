import { spawn, type ChildProcess } from 'node:child_process';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const START_TIMEOUT_MS = Number(process.env.FXVPS_GATEWAY_START_TIMEOUT_MS ?? 600_000);

/** Starts `client-gateway --demo --listen 127.0.0.1:0` and reads its URL + dev token from stdout. */
export default async function globalSetup(): Promise<() => Promise<void>> {
  const args = ['--demo', '--listen', '127.0.0.1:0'];
  const bin = process.env.FXVPS_GATEWAY_BIN;
  const child: ChildProcess = bin
    ? spawn(bin, args, { cwd: repoRoot, stdio: ['ignore', 'pipe', 'inherit'] })
    : spawn('cargo', ['run', '--quiet', '-p', 'client-gateway', '--', ...args], {
        cwd: repoRoot,
        stdio: ['ignore', 'pipe', 'inherit'],
        env: { ...process.env, RUST_LOG: process.env.RUST_LOG ?? 'warn' },
      });

  const found = await new Promise<{ url: string; token: string }>((resolve, reject) => {
    let url = '';
    let token = '';
    const timer = setTimeout(() => reject(new Error('client-gateway did not report FXVPS_WS_URL/FXVPS_DEMO_TOKEN in time')), START_TIMEOUT_MS);
    child.once('exit', (code) => {
      clearTimeout(timer);
      reject(new Error(`client-gateway exited early (code ${code})`));
    });
    createInterface({ input: child.stdout! }).on('line', (line) => {
      // eslint-disable-next-line no-control-regex
      const l = line.replace(/\x1b\[[0-9;]*m/g, '');
      const u = /FXVPS_WS_URL=(\S+)/.exec(l);
      const t = /FXVPS_DEMO_TOKEN=(\S+)/.exec(l);
      if (u) url = u[1]!;
      if (t) token = t[1]!;
      if (url && token) {
        clearTimeout(timer);
        resolve({ url, token });
      }
    });
  });
  process.env.FXVPS_WS_URL = found.url;
  process.env.FXVPS_DEMO_TOKEN = found.token;
  console.log(`client-gateway ready at ${found.url}`);

  return async () => {
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
  };
}

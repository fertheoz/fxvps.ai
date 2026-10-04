// Static export wired to the live core-engine (NEXT_PUBLIC_API_URL) into out-live/,
// leaving an existing mock build in out/ untouched.
import { spawnSync } from "node:child_process";
import { existsSync, renameSync, rmSync } from "node:fs";

const api = process.env.LIVE_API_URL ?? "http://127.0.0.1:8091";
const hadOut = existsSync("out");
if (hadOut) {
  rmSync("out.mock", { recursive: true, force: true });
  renameSync("out", "out.mock");
}
try {
  const r = spawnSync("npx", ["next", "build"], {
    stdio: "inherit",
    env: { ...process.env, NEXT_PUBLIC_API_URL: api, NEXT_PUBLIC_DEV_AUTH: "1" },
  });
  if (r.status !== 0) process.exit(r.status ?? 1);
  rmSync("out-live", { recursive: true, force: true });
  renameSync("out", "out-live");
} finally {
  if (hadOut) renameSync("out.mock", "out");
}

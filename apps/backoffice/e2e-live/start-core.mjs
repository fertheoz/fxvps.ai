// Starts core-engine for the live e2e: fresh data dir, dev auth, seeded demo
// data, CORS for the static back office. Uses $CORE_ENGINE_BIN when set
// (CI builds it first), otherwise `cargo run -p core-engine`.
import { spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const [addr = "127.0.0.1:8091", origin = "http://127.0.0.1:4311"] = process.argv.slice(2);
const repo = resolve(import.meta.dirname, "../../..");
const env = {
  ...process.env,
  CORE_DATA_DIR: mkdtempSync(join(tmpdir(), "core-e2e-")),
  CORE_ADMIN_ADDR: addr,
  CORE_DEV_AUTH: "1",
  CORE_SEED: "1",
  CORE_CORS_ORIGINS: origin,
  RUST_LOG: process.env.RUST_LOG ?? "info",
};
const bin = process.env.CORE_ENGINE_BIN;
const child = bin
  ? spawn(bin, [], { env, stdio: "inherit" })
  : spawn("cargo", ["run", "--locked", "-q", "-p", "core-engine"], { cwd: repo, env, stdio: "inherit" });
const stop = () => child.kill("SIGINT");
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
child.on("exit", (code) => process.exit(code ?? 0));

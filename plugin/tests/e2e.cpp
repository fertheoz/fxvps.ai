// End-to-end E1/E2 (docs/11): the C++ bridge core + fake MT5 server against a
// real fxvps client-gateway `/bridge` (demo engine).
//
//   e2e <ws-url> <institution> <key> [orders=1000] [pause_at=0]
//
// pause_at > 0: after that many orders the connection is cut with orders in
// flight; the core reconnects and re-sends them under the same ids. Success = every MT5 request answered, no request filled twice,
// and the server's reconciliation agrees with the fake MT5 net positions.
#include <chrono>
#include <cstdio>
#include <random>
#include <string>
#include <thread>

#include "../core/bridge.h"
#include "../net/ws_posix.h"
#include "../sim/sim_host.h"

using namespace fxvps;
using Clock = std::chrono::steady_clock;

static int64_t now_ms() {
  return std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now().time_since_epoch()).count();
}

int main(int argc, char** argv) {
  if (argc < 4) {
    std::fprintf(stderr, "usage: e2e <ws-url> <institution> <key> [orders] [pause_at]\n");
    return 2;
  }
  int total = argc > 4 ? std::atoi(argv[4]) : 1000;
  int pause_at = argc > 5 ? std::atoi(argv[5]) : 0;

  Config c;
  c.institution = argv[2];
  c.key = argv[3];
  c.server = "SIM" + std::to_string(now_ms() % 100000);  // fresh ids per run
  c.endpoints = {argv[1]};
  c.groups = {"real\\*", "pro\\*", "vip"};
  c.dead_ms = 3000;
  c.answer_timeout_ms = 30000;
  c.reconcile_every_ms = 3600000;

  SimHost mt5;
  PosixWs net;
  BridgeCore core(c, mt5, net);

  auto run_for = [&](int64_t ms, auto until) {
    int64_t end = now_ms() + ms;
    while (now_ms() < end) {
      core.poll(now_ms());
      if (until()) return true;
      std::this_thread::sleep_for(std::chrono::milliseconds(1));
    }
    return false;
  };

  if (!run_for(10000, [&] { return core.stats().connected; })) {
    std::printf("E2E FAIL: no session\n");
    for (auto& l : mt5.logs) std::printf("  %s\n", l.c_str());
    return 1;
  }
  run_for(500, [] { return false; });  // first quotes
  std::printf("connected: %zu symbols, %zu ticks\n", mt5.symbols.size(), mt5.tick_count);

  const char* groups[] = {"real\\std", "pro\\ecn", "vip"};
  const char* syms[] = {"EURUSD", "GBPUSD", "USDJPY"};
  const char* lots[] = {"0.01", "0.1", "0.25", "1"};
  std::mt19937 rng(42);
  int64_t t0 = now_ms();
  int sent = 0;
  uint64_t base = 1;
  bool paused = false;
  while (sent < total) {
    if (pause_at && sent == pause_at && !paused) {
      // E2: cut the connection while orders are in flight (network drop).
      paused = true;
      uint64_t sessions = core.stats().sessions;
      size_t in_flight = core.stats().pending;
      net.close();
      std::printf("connection cut at %d orders with %zu in flight\n", sent, in_flight);
      run_for(30000, [&] { return core.stats().sessions > sessions; });
      std::printf("session back: resends=%llu pending=%zu\n",
                  (unsigned long long)core.stats().resends, core.stats().pending);
    }
    if (!core.stats().connected) {  // a real MT5 would reject; the test waits
      core.poll(now_ms());
      std::this_thread::sleep_for(std::chrono::milliseconds(2));
      continue;
    }
    Request r;
    r.mt5_id = base + sent;
    r.login = 5000 + rng() % 100;
    r.group = groups[rng() % 3];
    r.symbol = syms[rng() % 3];
    r.side = rng() % 2 ? Side::Buy : Side::Sell;
    r.lots = *Dec::parse(lots[rng() % 4]);
    r.deviation = rng() % 2 ? 50 : 0;
    if (core.on_request(r) != Decision::Handled) {
      std::printf("E2E FAIL: request %llu not handled\n", (unsigned long long)r.mt5_id);
      return 1;
    }
    ++sent;
    core.poll(now_ms());
    // ~200 orders/s: under the institution quota
    if (sent % 20 == 0) run_for(100, [] { return false; });
  }
  bool all = run_for(30000, [&] { return mt5.outcomes() == static_cast<size_t>(total); });
  double secs = (now_ms() - t0) / 1000.0;

  size_t filled = 0, rejected = 0;
  for (int i = 0; i < total; ++i) {
    SimHost::Outcome o;
    if (mt5.outcome(base + i, o)) (o.filled ? filled : rejected)++;
  }
  core.reconcile_now();
  uint64_t ok0 = core.stats().reconcile_ok, diff0 = core.stats().reconcile_diff;
  run_for(5000, [&] { return core.stats().reconcile_ok > ok0 || core.stats().reconcile_diff > diff0; });
  Stats s = core.stats();
  bool rec_ok = s.reconcile_ok > ok0;

  std::printf("orders=%d answered=%zu filled=%zu rejected=%zu in %.1fs (%.0f/s)\n", total, mt5.outcomes(),
              filled, rejected, secs, total / secs);
  std::printf("confirms=%zu sessions=%llu resends=%llu quotes=%llu bad_frames=%llu reconcile=%s\n", mt5.confirms,
              (unsigned long long)s.sessions, (unsigned long long)s.resends, (unsigned long long)s.quotes,
              (unsigned long long)s.bad_frames, rec_ok ? "OK" : "DIFF");
  for (auto& kv : mt5.net_positions()) std::printf("  MT5 net %s %s\n", kv.first.c_str(), kv.second.str().c_str());
  for (auto& l : mt5.logs)
    if (l[0] != 'I') std::printf("  log: %s\n", l.c_str());

  bool pass = all && mt5.confirms == filled && rec_ok && s.bad_frames == 0 && rejected == 0;
  if (pause_at) pass = pass && s.sessions >= 2;
  std::printf(pass ? "E2E PASS\n" : "E2E FAIL\n");
  return pass ? 0 : 1;
}

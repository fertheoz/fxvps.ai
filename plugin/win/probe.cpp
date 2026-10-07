// Windows smoke test of the production transport: WinHTTP + bridge core +
// fake MT5 against a real /bridge endpoint.
//   bridge_probe wss://host/bridge <institution> <key> [orders=10]
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <thread>

#include "../core/bridge.h"
#include "../sim/sim_host.h"
#include "ws_winhttp.h"

using namespace fxvps;

static int64_t now_ms() {
  using namespace std::chrono;
  return duration_cast<milliseconds>(steady_clock::now().time_since_epoch()).count();
}

int main(int argc, char** argv) {
  if (argc < 4) {
    std::fprintf(stderr, "usage: bridge_probe <wss-url> <institution> <key> [orders]\n");
    return 2;
  }
  int orders = argc > 4 ? std::atoi(argv[4]) : 10;
  Config c;
  c.endpoints = {argv[1]};
  c.institution = argv[2];
  c.key = argv[3];
  c.server = "PROBE" + std::to_string(now_ms() % 100000);
  c.groups = {"*"};
  SimHost mt5;
  WinHttpWs net;
  BridgeCore core(c, mt5, net);
  auto run = [&](int64_t ms, auto until) {
    int64_t end = now_ms() + ms;
    while (now_ms() < end) {
      core.poll(now_ms());
      if (until()) return true;
      std::this_thread::sleep_for(std::chrono::milliseconds(2));
    }
    return false;
  };
  if (!run(15000, [&] { return core.stats().connected; })) {
    std::printf("PROBE FAIL: no session\n");
    for (auto& l : mt5.logs) std::printf("  %s\n", l.c_str());
    return 1;
  }
  for (int i = 0; i < orders; ++i) {
    Request r;
    r.mt5_id = static_cast<uint64_t>(i + 1);
    r.login = 1;
    r.group = "probe";
    r.symbol = "EURUSD";
    r.side = i % 2 ? Side::Sell : Side::Buy;
    r.lots = *Dec::parse("0.01");
    core.on_request(r);
  }
  run(15000, [&] { return mt5.outcomes() == static_cast<size_t>(orders); });
  std::printf("answered %zu/%d, confirms %zu, ticks %zu\n", mt5.outcomes(), orders, mt5.confirms, mt5.tick_count);
  return mt5.confirms == static_cast<size_t>(orders) ? 0 : 1;
}

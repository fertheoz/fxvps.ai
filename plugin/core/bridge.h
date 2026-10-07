// Bridge core: everything the MT5 plugin does, independent of MT5 and of the
// network library. Thread model: the MT5 adapter calls on_request() from MT5
// threads; one plugin worker thread calls poll() every few milliseconds. Host
// callbacks (confirm / reject / tick) run only inside poll(), outside the lock.
#pragma once
#include <cstdint>
#include <deque>
#include <functional>
#include <map>
#include <mutex>
#include <set>
#include <string>
#include <vector>

#include "host.h"
#include "json.h"
#include "transport.h"

namespace fxvps {

enum class Fallback { Reject, Local };

struct Config {
  std::string institution;
  std::string key;
  std::string server = "MT5";
  std::vector<std::string> endpoints;        // tried in turn (blue/green)
  std::vector<std::string> groups;           // managed groups, '*' wildcard
  std::map<std::string, std::string> symbols;  // MT5 name -> fxvps name
  std::string strip_suffix;                  // e.g. ".pro": EURUSD.pro -> EURUSD
  Fallback fallback = Fallback::Reject;
  int64_t heartbeat_ms = 1000;
  int64_t dead_ms = 3500;           // no frame from fxvps -> reconnect
  int64_t answer_timeout_ms = 60000;  // no final answer -> reject to MT5
  int64_t backoff_min_ms = 250;
  int64_t backoff_max_ms = 5000;
  int64_t reconcile_every_ms = 60000;

  // "key = value" lines; lists comma separated; '#' comments.
  // groups=real\*,pro\*  endpoints=wss://a/bridge,wss://b/bridge  symbol.XAUUSD.m=XAUUSD
  static bool parse(const std::string& text, Config& out, std::string& error);
};

enum class Decision {
  Handled,   // the bridge will confirm or reject it
  NotOurs,   // group not managed: MT5 processes it as usual
  Local,     // bridge down and fallback = Local: MT5 processes it
};

struct Stats {
  bool connected = false;
  uint64_t sessions = 0, orders = 0, fills = 0, rejects = 0, resends = 0, quotes = 0;
  uint64_t reconcile_ok = 0, reconcile_diff = 0, bad_frames = 0;
  size_t pending = 0;
};

bool group_matches(const std::string& pattern, const std::string& group);

class BridgeCore {
 public:
  BridgeCore(Config cfg, IHost& host, ITransport& net);

  Decision on_request(const Request& r);
  void poll(int64_t now_ms);
  // Requests a reconciliation at the next poll (also runs periodically).
  void reconcile_now();
  Stats stats() const;
  std::string order_id(uint64_t mt5_id) const;

 private:
  enum class St { Down, Connecting, Hello, Up };
  struct Pending {
    Request req;
    std::string id;
    bool sent = false;
    int64_t created_ms = 0;
    Dec filled;
  };
  using Action = std::function<void()>;

  void step(int64_t now, std::vector<Action>& out);
  void on_frame(const std::string& text, int64_t now, std::vector<Action>& out);
  void send_order(Pending& p);
  void send(const Json& j);
  void disconnect(int64_t now, const std::string& why, std::vector<Action>& out);
  void finish(const std::string& id, std::vector<Action>& out, std::function<void(IHost&, const Request&)> f);
  std::string to_fxvps(const std::string& mt5) const;
  bool managed(const std::string& group) const;

  Config cfg_;
  IHost& host_;
  ITransport& net_;
  mutable std::mutex mu_;
  St st_ = St::Down;
  size_t endpoint_ = 0;
  int64_t next_try_ms_ = 0, backoff_ms_ = 0, last_rx_ms_ = 0, last_ping_ms_ = 0, last_reconcile_ms_ = 0;
  int64_t now_ms_ = 0;
  bool reconcile_wanted_ = false;
  std::map<std::string, Pending> pending_;   // by order id
  std::deque<std::string> queue_;            // ids accepted while connecting
  std::map<std::string, std::string> to_mt5_;  // fxvps symbol -> MT5 symbol
  std::set<std::string> offered_;            // fxvps symbols in welcome
  Stats stats_;
};

}  // namespace fxvps

// Fake MT5 server for tests: logins, groups, request outcomes, tick table and
// per-login net positions built from confirmed requests (like MT5 deals).
#pragma once
#include <map>
#include <mutex>
#include <string>
#include <vector>

#include "../core/host.h"

namespace fxvps {

class SimHost : public IHost {
 public:
  struct Outcome {
    Request req;
    bool filled = false;
    Dec lots, price;
    RejectCode code = RejectCode::Rejected;
    std::string text;
  };

  void confirm(const Request& r, Dec filled, Dec price) override {
    std::lock_guard<std::mutex> lk(mu_);
    Outcome o{r, true, filled, price};
    outcomes_[r.mt5_id] = o;
    Dec signed_lots = r.side == Side::Buy ? filled : -filled;
    auto& pos = positions_[r.login][r.symbol];
    pos = pos + signed_lots;
    ++confirms;
  }
  void reject(const Request& r, RejectCode code, const std::string& text) override {
    std::lock_guard<std::mutex> lk(mu_);
    Outcome o{r, false, {}, {}, code, text};
    outcomes_[r.mt5_id] = o;
    ++rejects;
  }
  void tick(const std::string& sym, Dec bid, Dec ask, int64_t) override {
    std::lock_guard<std::mutex> lk(mu_);
    ticks_[sym] = {bid, ask};
    ++tick_count;
  }
  std::map<std::string, Dec> net_positions() override {
    std::lock_guard<std::mutex> lk(mu_);
    std::map<std::string, Dec> net;
    for (const auto& l : positions_)
      for (const auto& s : l.second) net[s.first] = net[s.first] + s.second;
    return net;
  }
  void log(LogLevel lvl, const std::string& text) override {
    std::lock_guard<std::mutex> lk(mu_);
    logs.push_back(std::string(lvl == LogLevel::Error ? "E " : lvl == LogLevel::Warn ? "W " : "I ") + text);
  }
  void on_symbols(const std::vector<SymbolInfo>& s) override {
    std::lock_guard<std::mutex> lk(mu_);
    symbols = s;
  }

  // test accessors
  bool outcome(uint64_t id, Outcome& o) {
    std::lock_guard<std::mutex> lk(mu_);
    auto it = outcomes_.find(id);
    if (it == outcomes_.end()) return false;
    o = it->second;
    return true;
  }
  size_t outcomes() {
    std::lock_guard<std::mutex> lk(mu_);
    return outcomes_.size();
  }
  bool last_tick(const std::string& sym, Dec& bid, Dec& ask) {
    std::lock_guard<std::mutex> lk(mu_);
    auto it = ticks_.find(sym);
    if (it == ticks_.end()) return false;
    bid = it->second.first;
    ask = it->second.second;
    return true;
  }
  void add_position(uint64_t login, const std::string& sym, Dec lots) {
    std::lock_guard<std::mutex> lk(mu_);
    positions_[login][sym] = positions_[login][sym] + lots;
  }

  size_t confirms = 0, rejects = 0, tick_count = 0;
  std::vector<std::string> logs;
  std::vector<SymbolInfo> symbols;

 private:
  std::mutex mu_;
  std::map<uint64_t, Outcome> outcomes_;
  std::map<uint64_t, std::map<std::string, Dec>> positions_;
  std::map<std::string, std::pair<Dec, Dec>> ticks_;
};

}  // namespace fxvps

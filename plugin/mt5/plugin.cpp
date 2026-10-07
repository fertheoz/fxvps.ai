// fxvps MT5 Server plugin — adapter skeleton (built only with FXVPS_MT5_SDK).
//
// The bridge core (plugin/core) holds all logic; this file only binds it to the
// MT5 Server API. Every "SDK:" comment marks a call whose exact name/signature
// must be taken from the SDK headers (MT5APIServer.h); nothing here is guessed
// into compiled code. See README.md for the responsibilities table.
#include <atomic>
#include <chrono>
#include <fstream>
#include <sstream>
#include <thread>

#include "../core/bridge.h"
#include "../win/ws_winhttp.h"

// SDK: #include "MT5APIServer.h"

namespace fxvps {

// IHost on top of the MT5 Server API. Called only from the worker thread.
class Mt5Host : public IHost {
 public:
  // explicit Mt5Host(IMTServerAPI* api) : api_(api) {}
  void confirm(const Request& r, Dec filled, Dec price) override {
    // SDK: answer the dealer request r.mt5_id as executed:
    //      confirm->Retcode(<done or done-partial>), ->Volume(lots->MT5 volume units),
    //      ->Price(price.to_double()); api_->DealerAnswer(confirm)
    (void)r; (void)filled; (void)price;
  }
  void reject(const Request& r, RejectCode code, const std::string& text) override {
    // SDK: answer the dealer request with a retcode mapped from `code`:
    //      Price -> price changed / requote-free "off quotes", Liquidity -> no money/no prices,
    //      Session|Timeout -> trade server busy / timeout, Margin -> no money, Rate -> too many requests
    (void)r; (void)code; (void)text;
  }
  void tick(const std::string& sym, Dec bid, Dec ask, int64_t ts_ns) override {
    // SDK: MTTick t{}; copy symbol, t.bid/t.ask = to_double(), t.datetime_msc = ts_ns/1e6;
    //      api_->TickAdd(sym.c_str(), t)
    (void)sym; (void)bid; (void)ask; (void)ts_ns;
  }
  std::map<std::string, Dec> net_positions() override {
    // SDK: for each managed group: positions of its logins (PositionGet*),
    //      sum signed volume per symbol, convert MT5 volume units -> lots.
    return {};
  }
  void log(LogLevel level, const std::string& text) override {
    // SDK: api_->LoggerOut(level == Error ? MTLogErr : MTLogOK, L"fxvps: %S", text.c_str())
    (void)level; (void)text;
  }
  // IMTServerAPI* api_ = nullptr;
};

// The plugin object MT5 creates (SDK: implements IMTServerPlugin + the
// request/dealer sink). Lifecycle: Start -> worker thread polls the core every
// 2 ms; Stop -> worker joins.
class Plugin {
 public:
  bool start(const std::string& config_path) {
    std::ifstream f(config_path);
    std::stringstream ss;
    ss << f.rdbuf();
    std::string err;
    Config cfg;
    if (!Config::parse(ss.str(), cfg, err)) {
      host_.log(LogLevel::Error, "config: " + err);
      return false;
    }
    core_ = std::make_unique<BridgeCore>(cfg, host_, net_);
    run_ = true;
    worker_ = std::thread([this] {
      using namespace std::chrono;
      while (run_) {
        core_->poll(duration_cast<milliseconds>(steady_clock::now().time_since_epoch()).count());
        std::this_thread::sleep_for(milliseconds(2));
      }
    });
    return true;
  }
  void stop() {
    run_ = false;
    if (worker_.joinable()) worker_.join();
    net_.close();
  }
  // SDK: request sink / dealer callback for a new trade request.
  //      Build Request from the MT5 request (login, group of the login, symbol,
  //      action/type -> side+kind, volume units -> lots, price, deviation),
  //      then: Decision d = core_->on_request(r);
  //      Handled -> the request stays with the plugin (answered by IHost),
  //      NotOurs/Local -> let MT5 process it as usual.
  Decision on_trade_request(const Request& r) { return core_ ? core_->on_request(r) : Decision::NotOurs; }

 private:
  Mt5Host host_;
  WinHttpWs net_;
  std::unique_ptr<BridgeCore> core_;
  std::atomic<bool> run_{false};
  std::thread worker_;
};

}  // namespace fxvps

// SDK: exported entry points MT5 loads (MTServerAbout / MTServerCreate) go here,
//      returning plugin info (name "fxvps bridge", version FXVPS_PLUGIN_VERSION,
//      parameter "config" = path of fxvps-bridge.conf) and a new fxvps::Plugin.

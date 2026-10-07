// What the bridge core needs from the MT5 server. The real implementation
// (plugin/mt5) binds these to the MT5 Server API; plugin/sim is a fake MT5
// used by every test.
#pragma once
#include <cstdint>
#include <map>
#include <string>
#include <vector>

#include "decimal.h"

namespace fxvps {

enum class Side { Buy, Sell };
enum class Kind { Market, Limit };

// A trade request captured by the MT5 adapter for a managed group.
struct Request {
  uint64_t mt5_id = 0;  // MT5 request id (unique per server run)
  uint64_t login = 0;
  std::string group;
  std::string symbol;   // MT5 symbol name
  Side side = Side::Buy;
  Dec lots;
  Kind kind = Kind::Market;
  Dec price;            // limit price (Kind::Limit)
  uint32_t deviation = 0;  // max slippage in points (0 = server default)
};

enum class RejectCode { Price, Liquidity, Rate, Session, BadRequest, Margin, Closed, Timeout, Rejected };
const char* to_string(RejectCode c);
RejectCode reject_code(const std::string& wire);

struct SymbolInfo {
  std::string symbol;  // fxvps name
  int digits = 5;
  Dec contract_size;
};

enum class LogLevel { Info, Warn, Error };

class IHost {
 public:
  virtual ~IHost() = default;
  // Fill: confirm the MT5 request with `filled` lots at average `price`.
  // `filled` < requested only for partially filled orders.
  virtual void confirm(const Request& r, Dec filled, Dec price) = 0;
  virtual void reject(const Request& r, RejectCode code, const std::string& text) = 0;
  virtual void tick(const std::string& mt5_symbol, Dec bid, Dec ask, int64_t ts_ns) = 0;
  // Signed net lots per MT5 symbol over the managed groups (reconciliation).
  virtual std::map<std::string, Dec> net_positions() = 0;
  virtual void log(LogLevel level, const std::string& text) = 0;
  // Symbols offered by fxvps (after each welcome).
  virtual void on_symbols(const std::vector<SymbolInfo>&) {}
};

}  // namespace fxvps

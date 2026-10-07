#include "bridge.h"

#include <algorithm>
#include <sstream>

namespace fxvps {

const char* to_string(RejectCode c) {
  switch (c) {
    case RejectCode::Price: return "price";
    case RejectCode::Liquidity: return "liquidity";
    case RejectCode::Rate: return "rate";
    case RejectCode::Session: return "session";
    case RejectCode::BadRequest: return "bad_request";
    case RejectCode::Margin: return "margin";
    case RejectCode::Closed: return "closed";
    case RejectCode::Timeout: return "timeout";
    case RejectCode::Rejected: return "rejected";
  }
  return "rejected";
}

RejectCode reject_code(const std::string& w) {
  if (w == "price") return RejectCode::Price;
  if (w == "liquidity") return RejectCode::Liquidity;
  if (w == "rate") return RejectCode::Rate;
  if (w == "session") return RejectCode::Session;
  if (w == "bad_request") return RejectCode::BadRequest;
  if (w == "margin") return RejectCode::Margin;
  if (w == "closed") return RejectCode::Closed;
  if (w == "timeout") return RejectCode::Timeout;
  return RejectCode::Rejected;
}

namespace {
std::string trim(const std::string& s) {
  size_t b = s.find_first_not_of(" \t\r\n"), e = s.find_last_not_of(" \t\r\n");
  return b == std::string::npos ? "" : s.substr(b, e - b + 1);
}
std::vector<std::string> split(const std::string& s) {
  std::vector<std::string> out;
  std::stringstream ss(s);
  std::string x;
  while (std::getline(ss, x, ',')) {
    x = trim(x);
    if (!x.empty()) out.push_back(x);
  }
  return out;
}
bool to_i64(const std::string& s, int64_t& v) {
  try {
    size_t n = 0;
    v = std::stoll(s, &n);
    return n == s.size() && v >= 0;
  } catch (...) {
    return false;
  }
}
}  // namespace

bool Config::parse(const std::string& text, Config& out, std::string& error) {
  Config c;
  std::stringstream ss(text);
  std::string line;
  int n = 0;
  while (std::getline(ss, line)) {
    ++n;
    line = trim(line);
    if (line.empty() || line[0] == '#') continue;
    size_t eq = line.find('=');
    if (eq == std::string::npos) { error = "line " + std::to_string(n) + ": key=value expected"; return false; }
    std::string k = trim(line.substr(0, eq)), v = trim(line.substr(eq + 1));
    int64_t num = 0;
    auto need_num = [&](int64_t& dst) {
      if (!to_i64(v, num)) { error = "line " + std::to_string(n) + ": number expected for " + k; return false; }
      dst = num;
      return true;
    };
    if (k == "institution") c.institution = v;
    else if (k == "key") c.key = v;
    else if (k == "server") c.server = v;
    else if (k == "endpoints") c.endpoints = split(v);
    else if (k == "groups") c.groups = split(v);
    else if (k == "strip_suffix") c.strip_suffix = v;
    else if (k == "fallback") {
      if (v == "reject") c.fallback = Fallback::Reject;
      else if (v == "local") c.fallback = Fallback::Local;
      else { error = "line " + std::to_string(n) + ": fallback must be reject or local"; return false; }
    } else if (k.rfind("symbol.", 0) == 0) c.symbols[k.substr(7)] = v;
    else if (k == "heartbeat_ms") { if (!need_num(c.heartbeat_ms)) return false; }
    else if (k == "dead_ms") { if (!need_num(c.dead_ms)) return false; }
    else if (k == "answer_timeout_ms") { if (!need_num(c.answer_timeout_ms)) return false; }
    else if (k == "backoff_min_ms") { if (!need_num(c.backoff_min_ms)) return false; }
    else if (k == "backoff_max_ms") { if (!need_num(c.backoff_max_ms)) return false; }
    else if (k == "reconcile_every_ms") { if (!need_num(c.reconcile_every_ms)) return false; }
    else { error = "line " + std::to_string(n) + ": unknown key " + k; return false; }
  }
  if (c.institution.empty() || c.key.empty()) { error = "institution and key are required"; return false; }
  if (c.endpoints.empty()) { error = "at least one endpoint is required"; return false; }
  if (c.groups.empty()) { error = "at least one managed group is required"; return false; }
  out = std::move(c);
  return true;
}

bool group_matches(const std::string& p, const std::string& g) {
  // iterative '*' glob, case-insensitive (MT5 group names are)
  auto low = [](char c) { return (c >= 'A' && c <= 'Z') ? char(c - 'A' + 'a') : c; };
  size_t pi = 0, gi = 0, star = std::string::npos, mark = 0;
  while (gi < g.size()) {
    if (pi < p.size() && p[pi] == '*') { star = pi++; mark = gi; }
    else if (pi < p.size() && low(p[pi]) == low(g[gi])) { ++pi; ++gi; }
    else if (star != std::string::npos) { pi = star + 1; gi = ++mark; }
    else return false;
  }
  while (pi < p.size() && p[pi] == '*') ++pi;
  return pi == p.size();
}

BridgeCore::BridgeCore(Config cfg, IHost& host, ITransport& net)
    : cfg_(std::move(cfg)), host_(host), net_(net) {}

std::string BridgeCore::order_id(uint64_t mt5_id) const {
  std::string s;
  for (char c : cfg_.server) s += (c == '~') ? '-' : c;
  if (s.size() > 40) s.resize(40);
  return s + "-" + std::to_string(mt5_id);
}

bool BridgeCore::managed(const std::string& group) const {
  for (const auto& p : cfg_.groups)
    if (group_matches(p, group)) return true;
  return false;
}

std::string BridgeCore::to_fxvps(const std::string& mt5) const {
  auto it = cfg_.symbols.find(mt5);
  if (it != cfg_.symbols.end()) return it->second;
  const auto& sfx = cfg_.strip_suffix;
  if (!sfx.empty() && mt5.size() > sfx.size() && mt5.compare(mt5.size() - sfx.size(), sfx.size(), sfx) == 0)
    return mt5.substr(0, mt5.size() - sfx.size());
  return mt5;
}

Decision BridgeCore::on_request(const Request& r) {
  std::lock_guard<std::mutex> lk(mu_);
  if (!managed(r.group)) return Decision::NotOurs;
  if (st_ != St::Up && cfg_.fallback == Fallback::Local) return Decision::Local;
  std::string id = order_id(r.mt5_id);
  if (pending_.count(id)) return Decision::Handled;  // MT5 re-delivered it
  Pending p;
  p.req = r;
  p.id = id;
  p.created_ms = now_ms_;
  auto& slot = pending_[id] = p;
  ++stats_.orders;
  if (st_ != St::Up) {
    queue_.push_back(id);  // rejected (session) at the next poll
    return Decision::Handled;
  }
  std::string sym = to_fxvps(r.symbol);
  if (!offered_.empty() && !offered_.count(sym)) {
    queue_.push_back(id);  // unknown symbol, rejected at the next poll
    return Decision::Handled;
  }
  send_order(slot);
  return Decision::Handled;
}

void BridgeCore::send(const Json& j) { net_.send(j.dump()); }

void BridgeCore::send_order(Pending& p) {
  const Request& r = p.req;
  Json o = Json::object();
  o.set("t", "order").set("id", p.id).set("login", Json::number(static_cast<long long>(r.login)));
  o.set("group", r.group).set("symbol", to_fxvps(r.symbol));
  o.set("side", r.side == Side::Buy ? "buy" : "sell").set("lots", r.lots.str());
  if (r.kind == Kind::Limit) o.set("kind", "limit").set("price", r.price.str());
  else o.set("kind", "market");
  if (r.deviation) o.set("deviation", Json::number(static_cast<long long>(r.deviation)));
  send(o);
  p.sent = true;
}

void BridgeCore::finish(const std::string& id, std::vector<Action>& out,
                        std::function<void(IHost&, const Request&)> f) {
  auto it = pending_.find(id);
  if (it == pending_.end()) return;
  Request r = it->second.req;
  pending_.erase(it);
  IHost* h = &host_;
  out.push_back([h, r, f] { f(*h, r); });
}

void BridgeCore::disconnect(int64_t now, const std::string& why, std::vector<Action>& out) {
  net_.close();
  bool was_up = st_ == St::Up;
  st_ = St::Down;
  stats_.connected = false;
  for (auto& kv : pending_) kv.second.sent = false;  // resent with the same id
  backoff_ms_ = backoff_ms_ ? std::min(backoff_ms_ * 2, cfg_.backoff_max_ms) : cfg_.backoff_min_ms;
  next_try_ms_ = now + backoff_ms_;
  if (!cfg_.endpoints.empty()) endpoint_ = (endpoint_ + 1) % cfg_.endpoints.size();
  IHost* h = &host_;
  std::string msg = std::string(was_up ? "fxvps session lost: " : "fxvps connect failed: ") + why +
                    " (retry in " + std::to_string(backoff_ms_) + " ms)";
  out.push_back([h, msg] { h->log(LogLevel::Warn, msg); });
}

void BridgeCore::on_frame(const std::string& text, int64_t now, std::vector<Action>& out) {
  Json m;
  if (!Json::parse(text, m) || !m.is_object()) {
    ++stats_.bad_frames;
    return;
  }
  last_rx_ms_ = now;
  std::string t = m["t"].str();
  IHost* h = &host_;
  if (t == "quote") {
    auto it = to_mt5_.find(m["s"].str());
    auto b = Dec::parse(m["b"].str()), a = Dec::parse(m["a"].str());
    if (it == to_mt5_.end() || !b || !a) return;
    ++stats_.quotes;
    std::string sym = it->second;
    Dec bid = *b, ask = *a;
    int64_t ts = m["ts"].integer(0);
    out.push_back([h, sym, bid, ask, ts] { h->tick(sym, bid, ask, ts); });
  } else if (t == "fill") {
    std::string id = m["id"].str();
    auto it = pending_.find(id);
    if (it == pending_.end()) return;
    auto filled = Dec::parse(m["filled"].str());
    if (filled) it->second.filled = *filled;
    if (!m["done"].truthy()) return;
    Dec f = it->second.filled;
    auto avg = Dec::parse(m["avg"].str());
    if (f.is_zero() || !avg) {
      ++stats_.rejects;
      finish(id, out, [](IHost& hh, const Request& r) { hh.reject(r, RejectCode::Rejected, "not filled"); });
    } else {
      ++stats_.fills;
      Dec px = *avg;
      finish(id, out, [f, px](IHost& hh, const Request& r) { hh.confirm(r, f, px); });
    }
  } else if (t == "reject") {
    ++stats_.rejects;
    RejectCode c = reject_code(m["code"].str());
    std::string txt = m["text"].str();
    finish(m["id"].str(), out, [c, txt](IHost& hh, const Request& r) { hh.reject(r, c, txt); });
  } else if (t == "welcome" && st_ == St::Hello) {
    st_ = St::Up;
    stats_.connected = true;
    ++stats_.sessions;
    backoff_ms_ = 0;
    offered_.clear();
    to_mt5_.clear();
    std::vector<SymbolInfo> syms;
    for (const auto& s : m["symbols"].items()) {
      SymbolInfo si;
      si.symbol = s["symbol"].str();
      si.digits = static_cast<int>(s["digits"].integer(5));
      if (auto cs = Dec::parse(s["contract_size"].str())) si.contract_size = *cs;
      offered_.insert(si.symbol);
      syms.push_back(si);
    }
    // fxvps -> MT5 names: explicit map first, then identity (+ suffix)
    for (const auto& s : offered_) to_mt5_[s] = s + cfg_.strip_suffix;
    for (const auto& kv : cfg_.symbols) to_mt5_[kv.second] = kv.first;
    out.push_back([h, syms] { h->on_symbols(syms); });
    std::string msg = "fxvps session up (" + std::to_string(syms.size()) + " symbols)";
    out.push_back([h, msg] { h->log(LogLevel::Info, msg); });
    for (auto& kv : pending_) {
      if (!kv.second.sent) {
        ++stats_.resends;
        send_order(kv.second);
      }
    }
    reconcile_wanted_ = true;
  } else if (t == "reconcile_result") {
    if (m["ok"].truthy()) ++stats_.reconcile_ok;
    else {
      ++stats_.reconcile_diff;
      std::string msg = "reconciliation mismatch (MT5 - fxvps): " + m["diff"].dump();
      out.push_back([h, msg] { h->log(LogLevel::Error, msg); });
    }
  } else if (t == "error") {
    std::string msg = "fxvps error " + m["code"].str() + ": " + m["text"].str();
    out.push_back([h, msg] { h->log(LogLevel::Error, msg); });
    disconnect(now, "server error " + m["code"].str(), out);
    if (m["code"].str() == "auth" || m["code"].str() == "version") {
      backoff_ms_ = cfg_.backoff_max_ms;  // no hammering with a bad key
      next_try_ms_ = now + backoff_ms_;
    }
  } else if (t == "pong") {
    // liveness only
  } else {
    ++stats_.bad_frames;
  }
}

void BridgeCore::step(int64_t now, std::vector<Action>& out) {
  now_ms_ = now;
  // requests that arrived while down / with unknown symbols
  while (!queue_.empty()) {
    std::string id = queue_.front();
    queue_.pop_front();
    auto it = pending_.find(id);
    if (it == pending_.end() || it->second.sent) continue;
    bool up = st_ == St::Up;
    ++stats_.rejects;
    if (!up) {
      finish(id, out, [](IHost& hh, const Request& r) { hh.reject(r, RejectCode::Session, "liquidity bridge offline"); });
    } else {
      std::string sym = to_fxvps(it->second.req.symbol);
      finish(id, out, [sym](IHost& hh, const Request& r) { hh.reject(r, RejectCode::BadRequest, "symbol not offered: " + sym); });
    }
  }

  switch (st_) {
    case St::Down:
      if (now >= next_try_ms_ && !cfg_.endpoints.empty()) {
        net_.open(cfg_.endpoints[endpoint_]);
        st_ = St::Connecting;
        last_rx_ms_ = now;
      }
      break;
    case St::Connecting: {
      auto s = net_.state();
      if (s == ITransport::State::Open) {
        Json h = Json::object();
        h.set("t", "hello").set("v", Json::number(1LL)).set("institution", cfg_.institution);
        h.set("key", cfg_.key).set("server", cfg_.server).set("plugin", FXVPS_PLUGIN_VERSION);
        send(h);
        st_ = St::Hello;
        last_rx_ms_ = now;
      } else if (s == ITransport::State::Failed || now - last_rx_ms_ > 10000) {
        disconnect(now, "connect", out);
      }
      break;
    }
    case St::Hello:
    case St::Up: {
      std::string text;
      int budget = 10000;  // frames per poll
      while (budget-- > 0 && net_.recv(text)) {
        on_frame(text, now, out);
        if (st_ == St::Down) break;
      }
      if (st_ == St::Down) break;
      if (net_.state() != ITransport::State::Open) {
        disconnect(now, "transport closed", out);
        break;
      }
      if (now - last_rx_ms_ > cfg_.dead_ms) {
        disconnect(now, "no traffic for " + std::to_string(now - last_rx_ms_) + " ms", out);
        break;
      }
      if (st_ == St::Up && now - last_ping_ms_ >= cfg_.heartbeat_ms) {
        Json p = Json::object();
        p.set("t", "ping").set("ts", Json::number(static_cast<long long>(now)));
        send(p);
        last_ping_ms_ = now;
      }
      break;
    }
  }

  // an MT5 request cannot wait forever
  std::vector<std::string> late;
  for (const auto& kv : pending_)
    if (now - kv.second.created_ms > cfg_.answer_timeout_ms) late.push_back(kv.first);
  for (const auto& id : late) {
    ++stats_.rejects;
    IHost* h = &host_;
    std::string msg = "no answer for " + id + "; rejected to MT5 - reconcile!";
    out.push_back([h, msg] { h->log(LogLevel::Error, msg); });
    finish(id, out, [](IHost& hh, const Request& r) { hh.reject(r, RejectCode::Timeout, "no answer from liquidity"); });
    reconcile_wanted_ = true;
  }
  stats_.pending = pending_.size();
}

void BridgeCore::poll(int64_t now) {
  std::vector<Action> out;
  bool due;
  {
    std::lock_guard<std::mutex> lk(mu_);
    step(now, out);
    due = st_ == St::Up && (reconcile_wanted_ || now - last_reconcile_ms_ >= cfg_.reconcile_every_ms);
    // Orders in flight make both sides move: compare when quiet, but never
    // postpone for more than one period (a busy server still reconciles).
    if (due && !pending_.empty() && now - last_reconcile_ms_ < 2 * cfg_.reconcile_every_ms &&
        !(reconcile_wanted_ && now - last_reconcile_ms_ >= cfg_.reconcile_every_ms))
      due = false;
    if (due) last_reconcile_ms_ = now;
  }
  for (auto& a : out) a();
  if (!due) return;
  auto net = host_.net_positions();  // MT5 call: never under our lock
  std::lock_guard<std::mutex> lk(mu_);
  if (st_ != St::Up) return;
  reconcile_wanted_ = false;
  std::map<std::string, Dec> sum;
  for (const auto& kv : net) sum[to_fxvps(kv.first)] = sum[to_fxvps(kv.first)] + kv.second;
  Json n = Json::object();
  for (const auto& kv : sum) n.set(kv.first, kv.second.str());
  Json r = Json::object();
  r.set("t", "reconcile").set("net", n);
  send(r);
}

void BridgeCore::reconcile_now() {
  std::lock_guard<std::mutex> lk(mu_);
  reconcile_wanted_ = true;
}

Stats BridgeCore::stats() const {
  std::lock_guard<std::mutex> lk(mu_);
  return stats_;
}

}  // namespace fxvps

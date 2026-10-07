// Bridge core scenarios S1-S12 (docs/11-mt5-plugin-plan.md) against an
// in-memory transport and the fake MT5 host; time is driven by hand.
#include <cstdio>
#include <fstream>
#include <sstream>
#include <deque>
#include <functional>
#include <string>
#include <vector>

#include "../core/bridge.h"
#include "../core/decimal.h"
#include "../core/json.h"
#include "../sim/sim_host.h"

using namespace fxvps;

static int failures = 0, checks = 0;
#define CHECK(c)                                                         \
  do {                                                                   \
    ++checks;                                                            \
    if (!(c)) {                                                          \
      ++failures;                                                        \
      std::printf("  FAIL %s:%d  %s\n", __FILE__, __LINE__, #c);         \
    }                                                                    \
  } while (0)

class MemNet : public ITransport {
 public:
  void open(const std::string& url) override {
    urls.push_back(url);
    st = accept ? State::Open : State::Failed;
  }
  State state() const override { return st; }
  bool send(const std::string& t) override {
    if (st != State::Open) return false;
    out.push_back(t);
    return true;
  }
  bool recv(std::string& t) override {
    if (in.empty()) return false;
    t = in.front();
    in.pop_front();
    return true;
  }
  void close() override { st = State::Idle; }

  // test side
  void push(const std::string& t) { in.push_back(t); }
  Json pop() {
    Json j;
    if (out.empty()) return j;
    Json::parse(out.front(), j);
    out.pop_front();
    return j;
  }
  Json pop_type(const std::string& t) {
    while (!out.empty()) {
      Json j = pop();
      if (j["t"].str() == t) return j;
    }
    return Json();
  }
  void drop() { st = State::Failed; }

  bool accept = true;
  State st = State::Idle;
  std::deque<std::string> in, out;
  std::vector<std::string> urls;
};

static const char* kWelcome =
    R"({"t":"welcome","v":1,"account":"K-1","heartbeat_ms":1000,"symbols":[)"
    R"({"symbol":"EURUSD","digits":5,"contract_size":"100000"},{"symbol":"XAUUSD","digits":2,"contract_size":"100"}]})";

static Config cfg() {
  Config c;
  std::string err;
  bool ok = Config::parse(
      "institution=kurum1\nkey=k\nserver=MT5-Live\n"
      "endpoints=ws://blue/bridge, ws://green/bridge\n"
      "groups=real\\*, pro\\vip\n"
      "symbol.GOLD=XAUUSD\nstrip_suffix=.pro\n"
      "dead_ms=3000\nanswer_timeout_ms=20000\nreconcile_every_ms=600000\n",
      c, err);
  if (!ok) std::printf("config: %s\n", err.c_str());
  return c;
}

static Request req(uint64_t id, const std::string& sym, Side s, const char* lots,
                   const std::string& group = "real\\std", uint64_t login = 1001) {
  Request r;
  r.mt5_id = id;
  r.login = login;
  r.group = group;
  r.symbol = sym;
  r.side = s;
  r.lots = *Dec::parse(lots);
  return r;
}

struct Rig {
  SimHost host;
  MemNet net;
  BridgeCore core;
  int64_t t = 1000;
  explicit Rig(Config c = cfg()) : core(c, host, net) {}
  void up() {
    core.poll(t);                 // opens
    core.poll(t += 10);           // sends hello
    net.pop_type("hello");
    net.push(kWelcome);
    core.poll(t += 10);
    net.out.clear();              // drop the initial reconcile
  }
  void step(int64_t ms = 10) { core.poll(t += ms); }
};

static void s1_connect_and_groups() {
  std::puts("S1 bağlanma, hello, grup filtresi");
  Rig r;
  r.core.poll(r.t);
  r.core.poll(r.t += 10);
  Json h = r.net.pop_type("hello");
  CHECK(h["institution"].str() == "kurum1");
  CHECK(h["v"].integer() == 1);
  CHECK(h["server"].str() == "MT5-Live");
  CHECK(r.net.urls.size() == 1 && r.net.urls[0] == "ws://blue/bridge");
  r.net.push(kWelcome);
  r.step();
  CHECK(r.core.stats().connected);
  CHECK(r.host.symbols.size() == 2);
  CHECK(r.core.on_request(req(1, "EURUSD", Side::Buy, "0.1", "demo\\x")) == Decision::NotOurs);
  CHECK(r.core.on_request(req(2, "EURUSD", Side::Buy, "0.1", "REAL\\Std")) == Decision::Handled);
  CHECK(r.core.on_request(req(3, "EURUSD", Side::Buy, "0.1", "pro\\vip")) == Decision::Handled);
  CHECK(r.core.on_request(req(4, "EURUSD", Side::Buy, "0.1", "pro\\vip2")) == Decision::NotOurs);
  CHECK(group_matches("real\\*", "real\\a\\b"));
  CHECK(!group_matches("real\\*", "demo\\real"));
}

static void s2_full_fill() {
  std::puts("S2 piyasa emri tam dolum");
  Rig r;
  r.up();
  Request q = req(10, "EURUSD.pro", Side::Buy, "0.30");
  q.deviation = 20;
  CHECK(r.core.on_request(q) == Decision::Handled);
  Json o = r.net.pop_type("order");
  CHECK(o["id"].str() == "MT5-Live-10");
  CHECK(o["symbol"].str() == "EURUSD");  // suffix stripped
  CHECK(o["lots"].str() == "0.3");
  CHECK(o["side"].str() == "buy");
  CHECK(o["deviation"].integer() == 20);
  CHECK(o["login"].integer() == 1001);
  r.net.push(R"({"t":"fill","id":"MT5-Live-10","lots":"0.3","price":"1.08492","filled":"0.3","avg":"1.08492","done":true})");
  r.step();
  SimHost::Outcome out;
  CHECK(r.host.outcome(10, out) && out.filled);
  CHECK(out.lots == *Dec::parse("0.3") && out.price == *Dec::parse("1.08492"));
  CHECK(r.core.stats().pending == 0);
}

static void s3_partial_fill() {
  std::puts("S3 kısmi dolum: MT5'e ortalama fiyatla tek onay");
  Rig r;
  r.up();
  r.core.on_request(req(11, "EURUSD", Side::Sell, "1"));
  r.net.push(R"({"t":"fill","id":"MT5-Live-11","lots":"0.4","price":"1.1","filled":"0.4","avg":"1.1","done":false})");
  r.step();
  SimHost::Outcome out;
  CHECK(!r.host.outcome(11, out));  // not yet
  r.net.push(R"({"t":"fill","id":"MT5-Live-11","lots":"0.6","price":"1.09990","filled":"1","avg":"1.09994","done":true})");
  r.step();
  CHECK(r.host.outcome(11, out) && out.filled && out.lots == Dec::from_int(1));
  CHECK(out.price == *Dec::parse("1.09994"));
  CHECK(r.host.confirms == 1);
  // partially filled then cancelled: confirm what filled
  r.core.on_request(req(12, "EURUSD", Side::Buy, "2"));
  r.net.push(R"({"t":"fill","id":"MT5-Live-12","lots":"0.5","price":"1.1","filled":"0.5","avg":"1.1","done":true})");
  r.step();
  CHECK(r.host.outcome(12, out) && out.filled && out.lots == *Dec::parse("0.5"));
}

static void s4_reject() {
  std::puts("S4 ret kodları");
  Rig r;
  r.up();
  r.core.on_request(req(20, "EURUSD", Side::Buy, "0.1"));
  r.net.push(R"({"t":"reject","id":"MT5-Live-20","code":"price","text":"price moved beyond slippage"})");
  r.step();
  SimHost::Outcome out;
  CHECK(r.host.outcome(20, out) && !out.filled && out.code == RejectCode::Price);
  // unknown symbol after welcome: rejected locally, never sent
  r.net.out.clear();
  r.core.on_request(req(21, "BTCXYZ", Side::Buy, "0.1"));
  r.step();
  CHECK(r.host.outcome(21, out) && out.code == RejectCode::BadRequest);
  CHECK(r.net.pop_type("order").is_null());
  // fill done with nothing filled = reject
  r.core.on_request(req(22, "EURUSD", Side::Buy, "0.1"));
  r.net.push(R"({"t":"fill","id":"MT5-Live-22","lots":"0","filled":"0","avg":null,"done":true})");
  r.step();
  CHECK(r.host.outcome(22, out) && !out.filled);
}

static void s5_resend_after_reconnect() {
  std::puts("S5 yanıtsız emir: yeniden bağlanınca aynı kimlikle tekrar, tek onay");
  Rig r;
  r.up();
  r.core.on_request(req(30, "EURUSD", Side::Buy, "0.2"));
  CHECK(r.net.pop_type("order")["id"].str() == "MT5-Live-30");
  r.net.drop();
  r.step();  // transport gone -> down
  CHECK(!r.core.stats().connected);
  CHECK(r.core.stats().pending == 1);
  r.step(1000);  // backoff passed -> reconnect to the other endpoint
  CHECK(r.net.urls.back() == "ws://green/bridge");
  r.step();
  r.net.pop_type("hello");
  r.net.push(kWelcome);
  r.step();
  Json o = r.net.pop_type("order");
  CHECK(o["id"].str() == "MT5-Live-30");  // same id: the server will not execute twice
  CHECK(r.core.stats().resends == 1);
  // the server answers from its record (lots 0 = nothing new, filled total)
  r.net.push(R"({"t":"fill","id":"MT5-Live-30","lots":"0","filled":"0.2","avg":"1.08","done":true})");
  r.net.push(R"({"t":"fill","id":"MT5-Live-30","lots":"0","filled":"0.2","avg":"1.08","done":true})");
  r.step();
  CHECK(r.host.confirms == 1);
  // MT5 re-delivering the same request does not create a second order
  r.core.on_request(req(31, "EURUSD", Side::Buy, "0.1"));
  CHECK(r.core.on_request(req(31, "EURUSD", Side::Buy, "0.1")) == Decision::Handled);
  int orders = 0;
  while (!r.net.pop_type("order").is_null()) ++orders;
  CHECK(orders == 1);
}

static void s6_fallback() {
  std::puts("S6 bağlantı yok: reject ve local düşüş modları");
  {
    Rig r;  // never connected (fallback reject)
    r.net.accept = false;
    r.step();
    CHECK(r.core.on_request(req(40, "EURUSD", Side::Buy, "0.1")) == Decision::Handled);
    r.step();
    SimHost::Outcome out;
    CHECK(r.host.outcome(40, out) && out.code == RejectCode::Session);
  }
  {
    Config c = cfg();
    c.fallback = Fallback::Local;
    Rig r(c);
    r.net.accept = false;
    r.step();
    CHECK(r.core.on_request(req(41, "EURUSD", Side::Buy, "0.1")) == Decision::Local);
    CHECK(r.host.outcomes() == 0);
  }
}

static void s7_backoff() {
  std::puts("S7 artan bekleme ve uç değiştirme");
  Rig r;
  r.net.accept = false;
  std::vector<int64_t> tries;
  for (int i = 0; i < 4000; ++i) {
    size_t n = r.net.urls.size();
    r.step(10);
    if (r.net.urls.size() != n) tries.push_back(r.t);
  }
  CHECK(tries.size() >= 5);
  if (tries.size() >= 4) {
    CHECK(tries[2] - tries[1] > tries[1] - tries[0]);  // growing
  }
  bool alternates = r.net.urls.size() > 1 && r.net.urls[0] != r.net.urls[1];
  CHECK(alternates);
  int64_t max_gap = 0;
  for (size_t i = 1; i < tries.size(); ++i) max_gap = std::max(max_gap, tries[i] - tries[i - 1]);
  CHECK(max_gap <= 5000 + 20);
}

static void s8_heartbeat() {
  std::puts("S8 kalp atışı ve sessizlikte kopuş");
  Rig r;
  r.up();
  r.step(1000);
  CHECK(!r.net.pop_type("ping").is_null());
  r.net.push(R"({"t":"pong","ts":1})");
  r.step(1000);
  CHECK(r.core.stats().connected);
  r.step(2500);
  r.step(1000);  // > dead_ms without frames
  CHECK(!r.core.stats().connected);
}

static void s9_quotes() {
  std::puts("S9 fiyat: MT5 adıyla tick, bilinmeyen sembol yok sayılır");
  Rig r;
  r.up();
  r.net.push(R"({"t":"quote","s":"EURUSD","b":"1.08490","a":"1.08500","ts":5})");
  r.net.push(R"({"t":"quote","s":"XAUUSD","b":"4162.27","a":"4163.37","ts":6})");
  r.net.push(R"({"t":"quote","s":"NOPE","b":"1","a":"2","ts":7})");
  r.step();
  Dec b, a;
  CHECK(r.host.last_tick("EURUSD.pro", b, a) && b == *Dec::parse("1.0849") && a == *Dec::parse("1.085"));
  CHECK(r.host.last_tick("GOLD", b, a) && a == *Dec::parse("4163.37"));  // explicit map
  CHECK(r.host.tick_count == 2);
}

static void s10_reconcile() {
  std::puts("S10 mutabakat");
  Rig r;
  r.up();
  r.host.add_position(1, "EURUSD.pro", *Dec::parse("0.3"));
  r.host.add_position(2, "EURUSD", *Dec::parse("-0.1"));
  r.host.add_position(2, "GOLD", *Dec::parse("1"));
  r.core.reconcile_now();
  r.step();
  Json m = r.net.pop_type("reconcile");
  CHECK(m["net"]["EURUSD"].str() == "0.2");
  CHECK(m["net"]["XAUUSD"].str() == "1");
  r.net.push(R"({"t":"reconcile_result","ok":false,"ours":{"EURUSD":"0.3"},"diff":{"EURUSD":"-0.1"}})");
  r.step();
  CHECK(r.core.stats().reconcile_diff == 1);
  bool logged = false;
  for (auto& l : r.host.logs) logged |= l.find("reconciliation mismatch") != std::string::npos;
  CHECK(logged);
}

static void s11_decimals() {
  std::puts("S11 ondalık biçim");
  CHECK(Dec::parse("0.1").value() + Dec::parse("0.2").value() == Dec::parse("0.3").value());
  CHECK(Dec::from_double(0.1 + 0.2, 2).str() == "0.3");
  CHECK(Dec::from_double(1.084925, 5).str() == "1.08493");
  CHECK(Dec::from_double(-1.084925, 5).str() == "-1.08493");
  CHECK(Dec::from_double(0.99999999, 2).str() == "1");
  CHECK(Dec::from_double(4163.37, 2).str() == "4163.37");
  CHECK(Dec::parse("-12.50").value().str() == "-12.5");
  CHECK(!Dec::parse("1.123456789").has_value());
  CHECK(Dec::parse("1.123456780").has_value());
  CHECK(!Dec::parse("abc").has_value());
  CHECK(!Dec::parse("").has_value());
  CHECK(Dec::parse("100000").value().str() == "100000");
  CHECK(Dec::from_int(-3).str() == "-3");
}

static void s12_bad_input() {
  std::puts("S12 bozuk girdi oturumu düşürmez");
  Rig r;
  r.up();
  r.net.push("{not json");
  r.net.push(R"({"t":"martian"})");
  r.net.push(R"({"t":"fill","id":"unknown","done":true})");
  r.net.push(R"([1,2,3])");
  r.step();
  CHECK(r.core.stats().connected);
  CHECK(r.core.stats().bad_frames == 3);
  Json j;
  CHECK(Json::parse(R"({"a":"ç😀","b":[true,null,-1.5e3]})", j));
  CHECK(j["a"].str() == "\xc3\xa7\xf0\x9f\x98\x80");
  CHECK(j["b"].items().size() == 3);
  std::string deep(100, '[');
  CHECK(!Json::parse(deep, j));
}

static void s13_timeout_and_auth() {
  std::puts("S13 yanıt zaman aşımı, kimlik hatasında yavaşlama, yapılandırma hataları");
  Rig r;
  r.up();
  r.core.on_request(req(50, "EURUSD", Side::Buy, "0.1"));
  for (int i = 0; i < 30; ++i) {
    r.net.push(R"({"t":"pong"})");
    r.step(1000);
  }
  SimHost::Outcome out;
  CHECK(r.host.outcome(50, out) && out.code == RejectCode::Timeout);
  r.net.push(R"({"t":"error","code":"auth","text":"authentication failed"})");
  r.step();
  CHECK(!r.core.stats().connected);
  size_t n = r.net.urls.size();
  r.step(4000);
  CHECK(r.net.urls.size() == n);  // waits backoff_max after an auth error
  r.step(1100);
  CHECK(r.net.urls.size() == n + 1);
  Config c;
  std::string err;
  CHECK(!Config::parse("institution=x\nkey=y\ngroups=a\n", c, err) && err.find("endpoint") != std::string::npos);
  CHECK(!Config::parse("institution=x\nkey=y\nendpoints=ws://a\ngroups=a\nfallback=maybe\n", c, err));
  CHECK(!Config::parse("institution=x\nkey=y\nendpoints=ws://a\ngroups=a\nbogus=1\n", c, err));
}

static void s14_example_config() {
  std::puts("S14 örnek yapılandırma dosyası okunur");
#ifdef FXVPS_SRC_DIR
  std::ifstream f(std::string(FXVPS_SRC_DIR) + "/mt5/fxvps-bridge.conf.example");
  std::stringstream ss;
  ss << f.rdbuf();
  Config c;
  std::string err;
  CHECK(Config::parse(ss.str(), c, err));
  CHECK(c.endpoints.size() == 2 && c.groups.size() == 2 && c.symbols["GOLD"] == "XAUUSD");
  CHECK(c.fallback == Fallback::Reject && c.strip_suffix == ".pro");
#endif
}

static void s15_reconcile_waits_for_quiet() {
  std::puts("S15 yoldaki emir varken mutabakat bekler");
  Rig r;
  r.up();
  r.core.on_request(req(60, "EURUSD", Side::Buy, "0.1"));
  r.net.out.clear();
  r.core.reconcile_now();
  r.step();
  CHECK(r.net.pop_type("reconcile").is_null());
  r.net.push(R"({"t":"fill","id":"MT5-Live-60","lots":"0.1","price":"1.1","filled":"0.1","avg":"1.1","done":true})");
  r.step();
  r.step();
  CHECK(!r.net.pop_type("reconcile").is_null());
}

int main() {
  s1_connect_and_groups();
  s2_full_fill();
  s3_partial_fill();
  s4_reject();
  s5_resend_after_reconnect();
  s6_fallback();
  s7_backoff();
  s8_heartbeat();
  s9_quotes();
  s10_reconcile();
  s11_decimals();
  s12_bad_input();
  s13_timeout_and_auth();
  s14_example_config();
  s15_reconcile_waits_for_quiet();
  std::printf("%d/%d checks passed\n", checks - failures, checks);
  return failures ? 1 : 0;
}

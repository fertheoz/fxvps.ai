// WinHTTP WebSocket transport (ws:// and wss://; TLS, proxy and certificate
// checks by Windows). No third-party code: this is what the MT5 server DLL uses.
#pragma once
#include <atomic>
#include <deque>
#include <mutex>
#include <string>
#include <thread>

#include "../core/transport.h"

namespace fxvps {

class WinHttpWs : public ITransport {
 public:
  WinHttpWs() = default;
  WinHttpWs(const WinHttpWs&) = delete;
  WinHttpWs& operator=(const WinHttpWs&) = delete;
  ~WinHttpWs() override;
  void open(const std::string& url) override;  // connects on a worker thread
  State state() const override { return state_.load(); }
  bool send(const std::string& text) override;
  bool recv(std::string& text) override;
  void close() override;

 private:
  void run(std::string url);
  void cleanup_handles();

  std::atomic<State> state_{State::Idle};
  std::atomic<bool> stop_{false};
  std::thread worker_;
  std::mutex mu_;  // frames_ and handles
  std::deque<std::string> frames_;
  void* session_ = nullptr;
  void* connect_ = nullptr;
  void* ws_ = nullptr;
};

}  // namespace fxvps

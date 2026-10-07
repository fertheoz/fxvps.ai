// Plain-TCP WebSocket client (ws:// only) for Linux end-to-end tests. The
// production transport is plugin/win (WinHTTP, wss://).
#pragma once
#include <deque>
#include <string>

#include "../core/transport.h"

namespace fxvps {

class PosixWs : public ITransport {
 public:
  ~PosixWs() override { close(); }
  void open(const std::string& url) override;  // blocking connect + handshake
  State state() const override { return st_; }
  bool send(const std::string& text) override;
  bool recv(std::string& text) override;
  void close() override;

 private:
  bool pump();  // non-blocking read + frame parse; false on EOF/error
  bool write_all(const std::string& bytes);
  int fd_ = -1;
  State st_ = State::Idle;
  std::string buf_;
  std::deque<std::string> frames_;
  std::string partial_;  // fragmented message
};

}  // namespace fxvps

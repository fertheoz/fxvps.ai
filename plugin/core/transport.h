// WebSocket text transport, polled by the bridge core (never blocks an MT5
// callback). Implementations: plugin/win (WinHTTP, TLS), plugin/net (plain
// TCP, Linux tests), tests (in-memory).
#pragma once
#include <string>

namespace fxvps {

class ITransport {
 public:
  virtual ~ITransport() = default;
  // Starts a connection to ws:// or wss:// `url`. May complete later
  // (state() turns Open) or fail (state() turns Failed).
  virtual void open(const std::string& url) = 0;
  enum class State { Idle, Connecting, Open, Failed };
  virtual State state() const = 0;
  virtual bool send(const std::string& text) = 0;
  // Pops one received text frame; false when none is pending.
  virtual bool recv(std::string& text) = 0;
  virtual void close() = 0;
};

}  // namespace fxvps

#include "ws_posix.h"

#include <arpa/inet.h>
#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>

#include <cerrno>
#include <cstring>
#include <random>

namespace fxvps {

namespace {
bool parse_url(const std::string& url, std::string& host, std::string& port, std::string& path) {
  const std::string p = "ws://";
  if (url.rfind(p, 0) != 0) return false;
  std::string rest = url.substr(p.size());
  size_t slash = rest.find('/');
  std::string hp = slash == std::string::npos ? rest : rest.substr(0, slash);
  path = slash == std::string::npos ? "/" : rest.substr(slash);
  size_t colon = hp.rfind(':');
  host = colon == std::string::npos ? hp : hp.substr(0, colon);
  port = colon == std::string::npos ? "80" : hp.substr(colon + 1);
  return !host.empty();
}
}  // namespace

void PosixWs::open(const std::string& url) {
  close();
  st_ = State::Failed;
  std::string host, port, path;
  if (!parse_url(url, host, port, path)) return;
  addrinfo hints{}, *res = nullptr;
  hints.ai_socktype = SOCK_STREAM;
  if (getaddrinfo(host.c_str(), port.c_str(), &hints, &res) != 0 || !res) return;
  int fd = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
  if (fd < 0 || connect(fd, res->ai_addr, res->ai_addrlen) != 0) {
    if (fd >= 0) ::close(fd);
    freeaddrinfo(res);
    return;
  }
  freeaddrinfo(res);
  int one = 1;
  setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof one);
  fd_ = fd;
  std::string req = "GET " + path + " HTTP/1.1\r\nHost: " + host + ":" + port +
                    "\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                    "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
  if (!write_all(req)) { close(); st_ = State::Failed; return; }
  std::string head;
  char c;
  while (head.find("\r\n\r\n") == std::string::npos && head.size() < 8192) {
    ssize_t n = ::recv(fd_, &c, 1, 0);
    if (n <= 0) { close(); st_ = State::Failed; return; }
    head += c;
  }
  if (head.find(" 101 ") == std::string::npos) { close(); st_ = State::Failed; return; }
  fcntl(fd_, F_SETFL, fcntl(fd_, F_GETFL, 0) | O_NONBLOCK);
  st_ = State::Open;
}

bool PosixWs::write_all(const std::string& bytes) {
  size_t off = 0;
  while (off < bytes.size()) {
    ssize_t n = ::send(fd_, bytes.data() + off, bytes.size() - off, MSG_NOSIGNAL);
    if (n < 0) {
      if (errno == EAGAIN || errno == EWOULDBLOCK) {
        pollfd p{fd_, POLLOUT, 0};
        ::poll(&p, 1, 1000);
        continue;
      }
      return false;
    }
    off += static_cast<size_t>(n);
  }
  return true;
}

bool PosixWs::send(const std::string& text) {
  if (st_ != State::Open) return false;
  static thread_local std::mt19937 rng{std::random_device{}()};
  std::string f;
  f += static_cast<char>(0x81);  // FIN + text
  size_t n = text.size();
  if (n < 126) f += static_cast<char>(0x80 | n);
  else if (n < 65536) { f += static_cast<char>(0x80 | 126); f += static_cast<char>(n >> 8); f += static_cast<char>(n & 0xFF); }
  else { f += static_cast<char>(0x80 | 127); for (int i = 7; i >= 0; --i) f += static_cast<char>((uint64_t(n) >> (8 * i)) & 0xFF); }
  uint32_t m = rng();
  unsigned char mask[4] = {uint8_t(m), uint8_t(m >> 8), uint8_t(m >> 16), uint8_t(m >> 24)};
  f.append(reinterpret_cast<char*>(mask), 4);
  for (size_t i = 0; i < n; ++i) f += static_cast<char>(text[i] ^ mask[i % 4]);
  if (!write_all(f)) { st_ = State::Failed; return false; }
  return true;
}

bool PosixWs::pump() {
  char tmp[65536];
  while (true) {
    ssize_t n = ::recv(fd_, tmp, sizeof tmp, 0);
    if (n > 0) { buf_.append(tmp, static_cast<size_t>(n)); continue; }
    if (n == 0) return false;
    if (errno == EAGAIN || errno == EWOULDBLOCK) break;
    return false;
  }
  while (buf_.size() >= 2) {
    auto b = reinterpret_cast<const unsigned char*>(buf_.data());
    bool fin = b[0] & 0x80;
    int op = b[0] & 0x0F;
    uint64_t len = b[1] & 0x7F;
    size_t h = 2;
    if (len == 126) { if (buf_.size() < 4) break; len = (uint64_t(b[2]) << 8) | b[3]; h = 4; }
    else if (len == 127) { if (buf_.size() < 10) break; len = 0; for (int i = 0; i < 8; ++i) len = (len << 8) | b[2 + i]; h = 10; }
    bool masked = b[1] & 0x80;
    if (masked) h += 4;
    if (buf_.size() < h + len) break;
    std::string payload = buf_.substr(h, len);
    if (masked) for (size_t i = 0; i < len; ++i) payload[i] ^= buf_[h - 4 + i % 4];
    buf_.erase(0, h + len);
    if (op == 0x8) return false;  // close
    if (op == 0x9) {             // ping -> pong
      std::string f;
      f += static_cast<char>(0x8A);
      f += static_cast<char>(0x80 | payload.size());
      f.append("\0\0\0\0", 4);
      f += payload;
      write_all(f);
      continue;
    }
    if (op == 0x1 || op == 0x0) {
      partial_ += payload;
      if (fin) { frames_.push_back(partial_); partial_.clear(); }
    }
  }
  return true;
}

bool PosixWs::recv(std::string& text) {
  if (frames_.empty() && st_ == State::Open && !pump()) st_ = State::Failed;
  if (frames_.empty()) return false;
  text = std::move(frames_.front());
  frames_.pop_front();
  return true;
}

void PosixWs::close() {
  if (fd_ >= 0) ::close(fd_);
  fd_ = -1;
  if (st_ != State::Failed) st_ = State::Idle;
  buf_.clear();
  frames_.clear();
  partial_.clear();
}

}  // namespace fxvps

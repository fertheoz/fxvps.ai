#include "ws_winhttp.h"

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#include <winhttp.h>

#include <iterator>
#include <vector>

namespace fxvps {

namespace {
std::wstring widen(const std::string& s) {
  if (s.empty()) return std::wstring();
  int n = MultiByteToWideChar(CP_UTF8, 0, s.data(), static_cast<int>(s.size()), nullptr, 0);
  std::wstring w(static_cast<size_t>(n), L'\0');
  MultiByteToWideChar(CP_UTF8, 0, s.data(), static_cast<int>(s.size()), &w[0], n);
  return w;
}
}  // namespace

WinHttpWs::~WinHttpWs() { close(); }

void WinHttpWs::open(const std::string& url) {
  close();
  stop_ = false;
  state_ = State::Connecting;
  worker_ = std::thread([this, url] { run(url); });
}

void WinHttpWs::cleanup_handles() {
  std::lock_guard<std::mutex> lk(mu_);
  if (ws_) WinHttpCloseHandle(static_cast<HINTERNET>(ws_));
  if (connect_) WinHttpCloseHandle(static_cast<HINTERNET>(connect_));
  if (session_) WinHttpCloseHandle(static_cast<HINTERNET>(session_));
  ws_ = connect_ = session_ = nullptr;
}

void WinHttpWs::run(std::string url) {
  std::wstring wurl = widen(url);
  // ws:// -> http://, wss:// -> https:// for WinHttpCrackUrl
  bool secure = url.rfind("wss://", 0) == 0;
  if (secure) wurl.replace(0, 3, L"https");
  else if (url.rfind("ws://", 0) == 0) wurl.replace(0, 2, L"http");
  else { state_ = State::Failed; return; }

  URL_COMPONENTS uc{};
  uc.dwStructSize = sizeof uc;
  wchar_t host[256] = {0}, path[2048] = {0};
  uc.lpszHostName = host;
  uc.dwHostNameLength = static_cast<DWORD>(std::size(host));
  uc.lpszUrlPath = path;
  uc.dwUrlPathLength = static_cast<DWORD>(std::size(path));
  if (!WinHttpCrackUrl(wurl.c_str(), 0, 0, &uc)) { state_ = State::Failed; return; }

  HINTERNET session = WinHttpOpen(L"fxvps-mt5-plugin", WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                                  WINHTTP_NO_PROXY_NAME, WINHTTP_NO_PROXY_BYPASS, 0);
  HINTERNET conn = session ? WinHttpConnect(session, host, uc.nPort, 0) : nullptr;
  {
    std::lock_guard<std::mutex> lk(mu_);
    session_ = session;
    connect_ = conn;
  }
  if (!conn) { cleanup_handles(); state_ = State::Failed; return; }
  WinHttpSetTimeouts(session, 5000, 5000, 5000, 0);  // receive: blocking, no timeout
  HINTERNET req = WinHttpOpenRequest(conn, L"GET", path[0] ? path : L"/", nullptr, WINHTTP_NO_REFERER,
                                     WINHTTP_DEFAULT_ACCEPT_TYPES, secure ? WINHTTP_FLAG_SECURE : 0);
  bool ok = req && WinHttpSetOption(req, WINHTTP_OPTION_UPGRADE_TO_WEB_SOCKET, nullptr, 0) &&
            WinHttpSendRequest(req, WINHTTP_NO_ADDITIONAL_HEADERS, 0, nullptr, 0, 0, 0) &&
            WinHttpReceiveResponse(req, nullptr);
  DWORD status = 0, len = sizeof status;
  if (ok)
    ok = WinHttpQueryHeaders(req, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_HEADER_NAME_BY_INDEX,
                             &status, &len, WINHTTP_NO_HEADER_INDEX) &&
         status == 101;
  HINTERNET ws = ok ? WinHttpWebSocketCompleteUpgrade(req, 0) : nullptr;
  if (req) WinHttpCloseHandle(req);
  if (!ws || stop_) {
    if (ws) WinHttpCloseHandle(ws);
    cleanup_handles();
    state_ = State::Failed;
    return;
  }
  {
    std::lock_guard<std::mutex> lk(mu_);
    ws_ = ws;
  }
  state_ = State::Open;

  std::vector<char> buf(64 * 1024);
  std::string msg;
  while (!stop_) {
    DWORD read = 0;
    WINHTTP_WEB_SOCKET_BUFFER_TYPE type;
    DWORD err = WinHttpWebSocketReceive(ws, buf.data(), static_cast<DWORD>(buf.size()), &read, &type);
    if (err != NO_ERROR || type == WINHTTP_WEB_SOCKET_CLOSE_BUFFER_TYPE) break;
    msg.append(buf.data(), read);
    if (msg.size() > 1024 * 1024) break;  // protocol frames are small
    if (type == WINHTTP_WEB_SOCKET_UTF8_MESSAGE_BUFFER_TYPE) {
      std::lock_guard<std::mutex> lk(mu_);
      frames_.push_back(std::move(msg));
      msg.clear();
    } else if (type == WINHTTP_WEB_SOCKET_BINARY_MESSAGE_BUFFER_TYPE) {
      msg.clear();  // not used by protocol v1
    }
  }
  if (!stop_) state_ = State::Failed;
}

bool WinHttpWs::send(const std::string& text) {
  if (state_ != State::Open) return false;
  HINTERNET ws;
  {
    std::lock_guard<std::mutex> lk(mu_);
    ws = static_cast<HINTERNET>(ws_);
  }
  if (!ws) return false;
  DWORD err = WinHttpWebSocketSend(ws, WINHTTP_WEB_SOCKET_UTF8_MESSAGE_BUFFER_TYPE,
                                   const_cast<char*>(text.data()), static_cast<DWORD>(text.size()));
  if (err != NO_ERROR) {
    state_ = State::Failed;
    return false;
  }
  return true;
}

bool WinHttpWs::recv(std::string& text) {
  std::lock_guard<std::mutex> lk(mu_);
  if (frames_.empty()) return false;
  text = std::move(frames_.front());
  frames_.pop_front();
  return true;
}

void WinHttpWs::close() {
  stop_ = true;
  {
    std::lock_guard<std::mutex> lk(mu_);
    if (ws_) WinHttpWebSocketClose(static_cast<HINTERNET>(ws_), WINHTTP_WEB_SOCKET_SUCCESS_CLOSE_STATUS, nullptr, 0);
  }
  cleanup_handles();  // unblocks a pending receive
  if (worker_.joinable()) worker_.join();
  {
    std::lock_guard<std::mutex> lk(mu_);
    frames_.clear();
  }
  state_ = State::Idle;
}

}  // namespace fxvps

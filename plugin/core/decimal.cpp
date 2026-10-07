#include "decimal.h"

#include <cmath>
#include <cstdio>
#include <limits>

namespace fxvps {

std::optional<Dec> Dec::parse(const std::string& s) {
  size_t i = 0;
  bool neg = false;
  if (i < s.size() && (s[i] == '-' || s[i] == '+')) neg = s[i++] == '-';
  if (i >= s.size()) return std::nullopt;
  int64_t ip = 0, fp = 0;
  int fd = 0;
  bool any = false, dot = false;
  for (; i < s.size(); ++i) {
    char c = s[i];
    if (c == '.') {
      if (dot) return std::nullopt;
      dot = true;
      continue;
    }
    if (c < '0' || c > '9') return std::nullopt;
    any = true;
    if (!dot) {
      if (ip > (std::numeric_limits<int64_t>::max() / kOne - 9) / 10) return std::nullopt;
      ip = ip * 10 + (c - '0');
    } else {
      if (fd == kScale) {
        if (c != '0') return std::nullopt;  // more precision than we carry
        continue;
      }
      fp = fp * 10 + (c - '0');
      ++fd;
    }
  }
  if (!any) return std::nullopt;
  for (; fd < kScale; ++fd) fp *= 10;
  int64_t r = ip * kOne + fp;
  return raw(neg ? -r : r);
}

Dec Dec::from_double(double v, int digits) {
  if (digits < 0) digits = 0;
  if (digits > kScale) digits = kScale;
  if (!std::isfinite(v)) return Dec();
  // Round on the shortest decimal text (12 places), not on the binary value:
  // 1.084925 is stored as 1.08492499999..., MT5 users mean 1.084925.
  char buf[64];
  std::snprintf(buf, sizeof buf, "%.12f", std::fabs(v));
  std::string s(buf);
  size_t dot = s.find('.');
  std::string ip = s.substr(0, dot), fp = s.substr(dot + 1);
  int64_t i = std::stoll(ip), f = 0;
  for (int k = 0; k < digits; ++k) f = f * 10 + (fp[k] - '0');
  if (fp[digits] >= '5') {  // half away from zero
    ++f;
    int64_t lim = 1;
    for (int k = 0; k < digits; ++k) lim *= 10;
    if (f == lim) { f = 0; ++i; }
  }
  int64_t step = 1;
  for (int k = digits; k < kScale; ++k) step *= 10;
  int64_t r = i * kOne + f * step;
  return raw(v < 0 ? -r : r);
}

std::string Dec::str() const {
  int64_t r = r_;
  bool neg = r < 0;
  uint64_t u = neg ? static_cast<uint64_t>(-(r + 1)) + 1 : static_cast<uint64_t>(r);
  uint64_t ip = u / kOne, fp = u % kOne;
  std::string out = (neg ? "-" : "") + std::to_string(ip);
  if (fp) {
    std::string f = std::to_string(fp);
    f = std::string(kScale - f.size(), '0') + f;
    while (!f.empty() && f.back() == '0') f.pop_back();
    out += "." + f;
  }
  return out;
}

}  // namespace fxvps

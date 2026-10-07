// Fixed-point decimal (8 places) for the wire format: the protocol carries
// numbers as decimal strings, never as binary floating point.
#pragma once
#include <cstdint>
#include <optional>
#include <string>

namespace fxvps {

class Dec {
 public:
  static constexpr int kScale = 8;
  static constexpr int64_t kOne = 100000000;

  constexpr Dec() = default;
  static constexpr Dec raw(int64_t r) { Dec d; d.r_ = r; return d; }
  static Dec from_int(int64_t v) { return raw(v * kOne); }

  // Parses "-12.345"; at most 8 decimals; nullopt on garbage or overflow.
  static std::optional<Dec> parse(const std::string& s);
  // Rounds a double to `digits` decimals (half away from zero).
  static Dec from_double(double v, int digits = kScale);

  int64_t raw() const { return r_; }
  double to_double() const { return static_cast<double>(r_) / kOne; }
  bool is_zero() const { return r_ == 0; }
  bool positive() const { return r_ > 0; }
  std::string str() const;  // shortest form: "0.3", "-1", "1.08492"

  Dec operator+(Dec o) const { return raw(r_ + o.r_); }
  Dec operator-(Dec o) const { return raw(r_ - o.r_); }
  Dec operator-() const { return raw(-r_); }
  bool operator==(Dec o) const { return r_ == o.r_; }
  bool operator!=(Dec o) const { return r_ != o.r_; }
  bool operator<(Dec o) const { return r_ < o.r_; }

 private:
  int64_t r_ = 0;
};

}  // namespace fxvps

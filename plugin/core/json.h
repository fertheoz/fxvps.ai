// Minimal JSON for protocol v1: objects, arrays, strings, numbers (kept as
// text), booleans, null. No dependencies; inputs are bounded (64 KiB frames).
#pragma once
#include <map>
#include <memory>
#include <string>
#include <vector>

namespace fxvps {

class Json {
 public:
  enum class Type { Null, Bool, Number, String, Array, Object };

  Json() = default;
  static Json null() { return Json(); }
  static Json boolean(bool b) { Json j; j.t_ = Type::Bool; j.b_ = b; return j; }
  static Json number(const std::string& text) { Json j; j.t_ = Type::Number; j.s_ = text; return j; }
  static Json number(long long v) { return number(std::to_string(v)); }
  static Json string(const std::string& s) { Json j; j.t_ = Type::String; j.s_ = s; return j; }
  static Json array() { Json j; j.t_ = Type::Array; return j; }
  static Json object() { Json j; j.t_ = Type::Object; return j; }

  // Parses one document; returns false on malformed input (out untouched).
  static bool parse(const std::string& text, Json& out);
  std::string dump() const;

  Type type() const { return t_; }
  bool is_null() const { return t_ == Type::Null; }
  bool is_object() const { return t_ == Type::Object; }
  bool is_array() const { return t_ == Type::Array; }
  bool is_string() const { return t_ == Type::String; }

  // Lenient accessors: wrong type -> default.
  std::string str(const std::string& def = "") const;  // string or number text
  bool truthy() const { return t_ == Type::Bool && b_; }
  long long integer(long long def = 0) const;
  const Json& operator[](const std::string& key) const;
  const std::vector<Json>& items() const { return a_; }
  const std::map<std::string, Json>& fields() const { return o_; }

  Json& set(const std::string& key, Json v) { t_ = Type::Object; o_[key] = std::move(v); return *this; }
  Json& set(const std::string& key, const std::string& s) { return set(key, string(s)); }
  Json& set(const std::string& key, const char* s) { return set(key, string(s)); }
  Json& push(Json v) { t_ = Type::Array; a_.push_back(std::move(v)); return *this; }

 private:
  Type t_ = Type::Null;
  bool b_ = false;
  std::string s_;
  std::vector<Json> a_;
  std::map<std::string, Json> o_;
  friend struct JsonParser;
};

}  // namespace fxvps

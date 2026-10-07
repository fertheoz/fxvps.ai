#include "json.h"

#include <cstdlib>

namespace fxvps {

namespace {
const Json kNull;

void put_utf8(std::string& out, unsigned cp) {
  if (cp < 0x80) {
    out += static_cast<char>(cp);
  } else if (cp < 0x800) {
    out += static_cast<char>(0xC0 | (cp >> 6));
    out += static_cast<char>(0x80 | (cp & 0x3F));
  } else if (cp < 0x10000) {
    out += static_cast<char>(0xE0 | (cp >> 12));
    out += static_cast<char>(0x80 | ((cp >> 6) & 0x3F));
    out += static_cast<char>(0x80 | (cp & 0x3F));
  } else {
    out += static_cast<char>(0xF0 | (cp >> 18));
    out += static_cast<char>(0x80 | ((cp >> 12) & 0x3F));
    out += static_cast<char>(0x80 | ((cp >> 6) & 0x3F));
    out += static_cast<char>(0x80 | (cp & 0x3F));
  }
}

void escape(std::string& out, const std::string& s) {
  out += '"';
  for (unsigned char c : s) {
    switch (c) {
      case '"': out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\n': out += "\\n"; break;
      case '\r': out += "\\r"; break;
      case '\t': out += "\\t"; break;
      default:
        if (c < 0x20) {
          static const char* hx = "0123456789abcdef";
          out += "\\u00";
          out += hx[c >> 4];
          out += hx[c & 15];
        } else {
          out += static_cast<char>(c);
        }
    }
  }
  out += '"';
}

void dump_to(const Json& j, std::string& out) {
  switch (j.type()) {
    case Json::Type::Null: out += "null"; break;
    case Json::Type::Bool: out += j.truthy() ? "true" : "false"; break;
    case Json::Type::Number: out += j.str("0"); break;
    case Json::Type::String: escape(out, j.str()); break;
    case Json::Type::Array: {
      out += '[';
      bool first = true;
      for (const auto& v : j.items()) {
        if (!first) out += ',';
        first = false;
        dump_to(v, out);
      }
      out += ']';
      break;
    }
    case Json::Type::Object: {
      out += '{';
      bool first = true;
      for (const auto& kv : j.fields()) {
        if (!first) out += ',';
        first = false;
        escape(out, kv.first);
        out += ':';
        dump_to(kv.second, out);
      }
      out += '}';
      break;
    }
  }
}
}  // namespace

struct JsonParser {
  const std::string& s;
  size_t i = 0;
  int depth = 0;

  void ws() {
    while (i < s.size() && (s[i] == ' ' || s[i] == '\n' || s[i] == '\r' || s[i] == '\t')) ++i;
  }
  bool lit(const char* w) {
    size_t n = 0;
    while (w[n]) ++n;
    if (s.compare(i, n, w) != 0) return false;
    i += n;
    return true;
  }
  bool hex4(unsigned& v) {
    if (i + 4 > s.size()) return false;
    v = 0;
    for (int k = 0; k < 4; ++k) {
      char c = s[i++];
      v <<= 4;
      if (c >= '0' && c <= '9') v |= c - '0';
      else if (c >= 'a' && c <= 'f') v |= c - 'a' + 10;
      else if (c >= 'A' && c <= 'F') v |= c - 'A' + 10;
      else return false;
    }
    return true;
  }
  bool string(std::string& out) {
    if (i >= s.size() || s[i] != '"') return false;
    ++i;
    while (i < s.size()) {
      char c = s[i++];
      if (c == '"') return true;
      if (static_cast<unsigned char>(c) < 0x20) return false;
      if (c != '\\') { out += c; continue; }
      if (i >= s.size()) return false;
      char e = s[i++];
      switch (e) {
        case '"': out += '"'; break;
        case '\\': out += '\\'; break;
        case '/': out += '/'; break;
        case 'b': out += '\b'; break;
        case 'f': out += '\f'; break;
        case 'n': out += '\n'; break;
        case 'r': out += '\r'; break;
        case 't': out += '\t'; break;
        case 'u': {
          unsigned cp;
          if (!hex4(cp)) return false;
          if (cp >= 0xD800 && cp < 0xDC00) {  // surrogate pair
            unsigned lo;
            if (i + 2 > s.size() || s[i] != '\\' || s[i + 1] != 'u') return false;
            i += 2;
            if (!hex4(lo) || lo < 0xDC00 || lo > 0xDFFF) return false;
            cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
          }
          put_utf8(out, cp);
          break;
        }
        default: return false;
      }
    }
    return false;
  }
  bool number(std::string& out) {
    size_t b = i;
    if (i < s.size() && s[i] == '-') ++i;
    size_t d = i;
    while (i < s.size() && ((s[i] >= '0' && s[i] <= '9') || s[i] == '.' || s[i] == 'e' || s[i] == 'E' || s[i] == '+' || s[i] == '-')) ++i;
    if (i == d) return false;
    out = s.substr(b, i - b);
    return true;
  }
  bool value(Json& out) {
    if (++depth > 32) return false;
    ws();
    if (i >= s.size()) return false;
    char c = s[i];
    bool ok = false;
    if (c == '{') {
      ++i;
      out = Json::object();
      ws();
      if (i < s.size() && s[i] == '}') { ++i; ok = true; }
      else {
        while (true) {
          ws();
          std::string k;
          if (!string(k)) break;
          ws();
          if (i >= s.size() || s[i] != ':') break;
          ++i;
          Json v;
          if (!value(v)) break;
          out.o_[k] = std::move(v);
          ws();
          if (i < s.size() && s[i] == ',') { ++i; continue; }
          if (i < s.size() && s[i] == '}') { ++i; ok = true; }
          break;
        }
      }
    } else if (c == '[') {
      ++i;
      out = Json::array();
      ws();
      if (i < s.size() && s[i] == ']') { ++i; ok = true; }
      else {
        while (true) {
          Json v;
          if (!value(v)) break;
          out.a_.push_back(std::move(v));
          ws();
          if (i < s.size() && s[i] == ',') { ++i; continue; }
          if (i < s.size() && s[i] == ']') { ++i; ok = true; }
          break;
        }
      }
    } else if (c == '"') {
      std::string v;
      ok = string(v);
      if (ok) out = Json::string(v);
    } else if (lit("true")) { out = Json::boolean(true); ok = true; }
    else if (lit("false")) { out = Json::boolean(false); ok = true; }
    else if (lit("null")) { out = Json::null(); ok = true; }
    else {
      std::string v;
      ok = number(v);
      if (ok) out = Json::number(v);
    }
    --depth;
    return ok;
  }
};

bool Json::parse(const std::string& text, Json& out) {
  JsonParser p{text};
  Json v;
  if (!p.value(v)) return false;
  p.ws();
  if (p.i != text.size()) return false;
  out = std::move(v);
  return true;
}

std::string Json::dump() const {
  std::string out;
  dump_to(*this, out);
  return out;
}

std::string Json::str(const std::string& def) const {
  return (t_ == Type::String || t_ == Type::Number) ? s_ : def;
}

long long Json::integer(long long def) const {
  if (t_ != Type::Number && t_ != Type::String) return def;
  char* end = nullptr;
  long long v = std::strtoll(s_.c_str(), &end, 10);
  return (end && *end == '\0' && !s_.empty()) ? v : def;
}

const Json& Json::operator[](const std::string& key) const {
  if (t_ != Type::Object) return kNull;
  auto it = o_.find(key);
  return it == o_.end() ? kNull : it->second;
}

}  // namespace fxvps

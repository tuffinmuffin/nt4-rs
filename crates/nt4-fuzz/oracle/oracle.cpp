// The oracle: ntcore's own decoders (from the pyntcore wheel's libntcore),
// behind a C API, so nt4-fuzz can compare nt4-proto with them on the same
// bytes. Each function returns a JSON string (free it with nt4_oracle_free)
// in the canonical form nt4-fuzz builds for nt4-proto's output:
//
//   binary: {"msgs": [{"id": "1", "time": "5", "value": V}, ...],
//            "error": null or ntcore's error text as hex}
//   text:   {"calls": [handler calls in order], "warnings": [log lines, hex]}
//
// Either gets "exception": what() as hex if ntcore threw.
//
// V is {"t": type, "v": ...}. Numbers are decimal strings, doubles and floats
// their bit patterns in hex, and every string is hex, so nothing is lost in
// JSON and nothing here needs escaping. Properties are JSON values in the
// form Canon() below writes. Times are microseconds, as on the wire (ntcore
// holds them in nanoseconds). Built by build.rs when NT4_NTCORE_PYTHON is
// set.
//
// Written by Claude.

#include <algorithm>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include <wpi/nt/NetworkTableValue.hpp>
#include <wpi/util/Logger.hpp>
#include <wpi/util/json.hpp>

#include "PubSubOptions.hpp"
#include "net/MessageHandler.hpp"
#include "net/WireDecoder.hpp"

namespace {

using wpi::util::json;

std::string Hex(std::string_view s) {
  static const char* digits = "0123456789abcdef";
  std::string out;
  out.reserve(s.size() * 2 + 2);
  out.push_back('"');
  for (unsigned char c : s) {
    out.push_back(digits[c >> 4]);
    out.push_back(digits[c & 15]);
  }
  out.push_back('"');
  return out;
}

std::string Hex(std::span<const uint8_t> b) {
  return Hex(std::string_view{reinterpret_cast<const char*>(b.data()), b.size()});
}

std::string Bits(double d) {
  return Hex(std::string_view{reinterpret_cast<const char*>(&d), 8});
}

std::string Bits(float f) {
  return Hex(std::string_view{reinterpret_cast<const char*>(&f), 4});
}

std::string Dec(long long v) {
  return '"' + std::to_string(v) + '"';
}

const char* Bool(bool b) {
  return b ? "true" : "false";
}

template <typename T, typename F>
std::string List(std::span<const T> items, F f) {
  std::string out = "[";
  bool first = true;
  for (const auto& i : items) {
    if (!first) {
      out.push_back(',');
    }
    first = false;
    out += f(i);
  }
  out.push_back(']');
  return out;
}

// A JSON value (properties) with nothing left to the printer: integers as
// {"i": decimal}, other numbers as {"f": double bits}, strings as
// {"s": hex}, objects as {"o": [[hex key, value], ...]} sorted by key.
// Iterative on arrays would be nicer, but ntcore's parser stops at 20
// levels, so recursion is shallow.
std::string Canon(const json& j) {
  switch (j.type()) {
    case json::Type::Null:
      return "null";
    case json::Type::Bool:
      return Bool(j.get_bool());
    case json::Type::Int:
      return "{\"i\":" + Dec(j.get_int()) + "}";
    case json::Type::Uint:
      return "{\"u\":\"" + std::to_string(j.get_uint()) + "\"}";
    case json::Type::Float:
    case json::Type::Double:
      return "{\"f\":" + Bits(j.get_number()) + "}";
    case json::Type::String:
      return "{\"s\":" + Hex(j.get_string()) + "}";
    case json::Type::Array:
      return List(std::span<const json>{j.get_array()},
                  [](const json& v) { return Canon(v); });
    case json::Type::Object: {
      std::vector<std::pair<std::string, std::string>> items;
      for (auto&& kv : j.get_object()) {
        items.emplace_back(std::string{kv.first}, Canon(kv.second));
      }
      std::sort(items.begin(), items.end());
      std::string out = "{\"o\":[";
      bool first = true;
      for (auto& [k, v] : items) {
        if (!first) {
          out.push_back(',');
        }
        first = false;
        out += "[" + Hex(k) + "," + v + "]";
      }
      return out + "]}";
    }
  }
  return "null";
}

std::string ValueJson(const wpi::nt::Value& v) {
  auto tv = [](const char* t, const std::string& val) {
    return std::string{"{\"t\":\""} + t + "\",\"v\":" + val + "}";
  };
  auto quoted = [](const char* s) { return std::string{"\""} + s + "\""; };
  switch (v.type()) {
    case NT_BOOLEAN:
      return tv("boolean", quoted(Bool(v.GetBoolean())));
    case NT_DOUBLE:
      return tv("double", Bits(v.GetDouble()));
    case NT_INTEGER:
      return tv("int", Dec(v.GetInteger()));
    case NT_FLOAT:
      return tv("float", Bits(v.GetFloat()));
    case NT_STRING:
      return tv("string", Hex(v.GetString()));
    case NT_RAW:
      return tv("raw", Hex(v.GetRaw()));
    case NT_BOOLEAN_ARRAY:
      return tv("boolean[]", List(v.GetBooleanArray(), [&](int b) {
                  return quoted(Bool(b != 0));
                }));
    case NT_DOUBLE_ARRAY:
      return tv("double[]",
                List(v.GetDoubleArray(), [](double d) { return Bits(d); }));
    case NT_INTEGER_ARRAY:
      return tv("int[]",
                List(v.GetIntegerArray(), [](int64_t i) { return Dec(i); }));
    case NT_FLOAT_ARRAY:
      return tv("float[]",
                List(v.GetFloatArray(), [](float f) { return Bits(f); }));
    case NT_STRING_ARRAY:
      return tv("string[]", List(v.GetStringArray(), [](const std::string& s) {
                  return Hex(s);
                }));
    default:
      return "{\"t\":\"unassigned\"}";
  }
}

char* Finish(const std::string& s) {
  char* out = static_cast<char*>(std::malloc(s.size() + 1));
  std::memcpy(out, s.c_str(), s.size() + 1);
  return out;
}

std::string Join(const std::vector<std::string>& items) {
  std::string out = "[";
  for (size_t i = 0; i < items.size(); ++i) {
    if (i > 0) {
      out.push_back(',');
    }
    out += items[i];
  }
  return out + "]";
}

std::string OptionsJson(const wpi::nt::PubSubOptionsImpl& o) {
  return "{\"periodic\":" + Bits(o.periodic) + ",\"all\":" + Bool(o.sendAll) +
         ",\"topicsOnly\":" + Bool(o.topicsOnly) + ",\"prefix\":" +
         Bool(o.prefixMatch) + "}";
}

struct Recorder {
  std::vector<std::string> calls;
  std::vector<std::string> warnings;
  wpi::util::Logger logger;

  Recorder() {
    logger.SetLogger([this](unsigned int, const char*, unsigned int,
                            const char* msg) { warnings.push_back(Hex(msg)); });
    logger.set_min_level(0);
  }

  std::string Result() const {
    return "{\"calls\":" + Join(calls) + ",\"warnings\":" + Join(warnings);
  }
};

class ClientHandler : public wpi::nt::net::ClientMessageHandler {
 public:
  explicit ClientHandler(Recorder& r) : r{r} {}
  void ClientPublish(int pubuid, std::string_view name,
                     std::string_view typeStr, const json& properties,
                     const wpi::nt::PubSubOptionsImpl&) override {
    r.calls.push_back("{\"m\":\"publish\",\"pubuid\":" + Dec(pubuid) +
                      ",\"name\":" + Hex(name) + ",\"type\":" + Hex(typeStr) +
                      ",\"props\":" + Canon(properties) + "}");
  }
  void ClientUnpublish(int pubuid) override {
    r.calls.push_back("{\"m\":\"unpublish\",\"pubuid\":" + Dec(pubuid) + "}");
  }
  void ClientSetProperties(std::string_view name, const json& update) override {
    r.calls.push_back("{\"m\":\"setproperties\",\"name\":" + Hex(name) +
                      ",\"update\":" + Canon(update) + "}");
  }
  void ClientSubscribe(int subuid, std::span<const std::string> topicNames,
                       const wpi::nt::PubSubOptionsImpl& options) override {
    r.calls.push_back(
        "{\"m\":\"subscribe\",\"subuid\":" + Dec(subuid) + ",\"topics\":" +
        List(topicNames, [](const std::string& s) { return Hex(s); }) +
        ",\"options\":" + OptionsJson(options) + "}");
  }
  void ClientUnsubscribe(int subuid) override {
    r.calls.push_back("{\"m\":\"unsubscribe\",\"subuid\":" + Dec(subuid) + "}");
  }
  void ClientSetValue(int, const wpi::nt::Value&) override {}

 private:
  Recorder& r;
};

class ServerHandler : public wpi::nt::net::ServerMessageHandler {
 public:
  explicit ServerHandler(Recorder& r) : r{r} {}
  int ServerAnnounce(std::string_view name, int id, std::string_view typeStr,
                     const json& properties,
                     std::optional<int> pubuid) override {
    r.calls.push_back("{\"m\":\"announce\",\"name\":" + Hex(name) +
                      ",\"id\":" + Dec(id) + ",\"type\":" + Hex(typeStr) +
                      ",\"pubuid\":" + (pubuid ? Dec(*pubuid) : "null") +
                      ",\"props\":" + Canon(properties) + "}");
    return 0;
  }
  void ServerUnannounce(std::string_view name, int id) override {
    r.calls.push_back("{\"m\":\"unannounce\",\"name\":" + Hex(name) +
                      ",\"id\":" + Dec(id) + "}");
  }
  void ServerPropertiesUpdate(std::string_view name, const json& update,
                              bool ack) override {
    r.calls.push_back("{\"m\":\"properties\",\"name\":" + Hex(name) +
                      ",\"update\":" + Canon(update) + ",\"ack\":" + Bool(ack) +
                      "}");
  }
  void ServerSetValue(int, const wpi::nt::Value&) override {}

 private:
  Recorder& r;
};

template <typename Handler>
char* DecodeText(const char* data, size_t len) {
  Recorder r;
  Handler h{r};
  // An exception here would end an ntcore process; report it instead.
  try {
    wpi::nt::net::WireDecodeText({data, len}, h, r.logger);
  } catch (const std::exception& e) {
    return Finish(r.Result() + ",\"exception\":" + Hex(e.what()) + "}");
  }
  return Finish(r.Result() + "}");
}

}  // namespace

extern "C" {

char* nt4_oracle_binary(const uint8_t* data, size_t len) {
  std::span<const uint8_t> in{data, len};
  std::vector<std::string> msgs;
  std::string error = "null";
  // As ntcore's ClientImpl and ServerImpl read a frame: until it is empty or
  // a message fails.
  while (!in.empty()) {
    int id = 0;
    wpi::nt::Value value;
    std::string err;
    try {
      if (!wpi::nt::net::WireDecodeBinary(&in, &id, &value, &err, 0)) {
        error = Hex(err);
        break;
      }
    } catch (const std::exception& e) {
      return Finish("{\"msgs\":" + Join(msgs) +
                    ",\"error\":null,\"exception\":" + Hex(e.what()) + "}");
    }
    // ntcore holds times in nanoseconds: wire microseconds times 1000.
    msgs.push_back("{\"id\":" + Dec(id) + ",\"time\":" +
                   Dec(value.server_time() / 1000) + ",\"value\":" +
                   ValueJson(value) + "}");
  }
  return Finish("{\"msgs\":" + Join(msgs) + ",\"error\":" + error + "}");
}

char* nt4_oracle_client_text(const char* data, size_t len) {
  return DecodeText<ClientHandler>(data, len);
}

char* nt4_oracle_server_text(const char* data, size_t len) {
  return DecodeText<ServerHandler>(data, len);
}

void nt4_oracle_free(char* s) { std::free(s); }

}  // extern "C"

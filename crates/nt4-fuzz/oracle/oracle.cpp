// The oracle: ntcore's own decoders (from the pyntcore wheel's libntcore),
// behind a C API, so nt4-fuzz can compare nt4-proto with them on the same
// bytes. Each function returns a JSON string (free it with nt4_oracle_free)
// in the canonical form nt4-fuzz builds for nt4-proto's output:
//
//   binary: {"msgs": [{"id": "1", "time": "5", "value": V}, ...],
//            "error": null or ntcore's error text}
//   text:   {"calls": [handler calls in order], "warnings": [log lines]}
//
// Either gets "exception": what() if ntcore threw.
//
// V is {"t": type, "v": ...}. Numbers are decimal strings, doubles and floats
// their bit patterns in hex, strings and raw bytes hex, so nothing is lost in
// JSON. Strings in text frames (names, type strings) are hex for the same
// reason. Built by build.rs when NT4_NTCORE_PYTHON is set.
//
// Written by Claude.

#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

#include <networktables/NetworkTableValue.h>
#include <wpi/Logger.h>
#include <wpi/json.h>

#include "PubSubOptions.h"
#include "net/MessageHandler.h"
#include "net/WireDecoder.h"

namespace {

std::string Hex(std::string_view s) {
  static const char* digits = "0123456789abcdef";
  std::string out;
  out.reserve(s.size() * 2);
  for (unsigned char c : s) {
    out.push_back(digits[c >> 4]);
    out.push_back(digits[c & 15]);
  }
  return out;
}

std::string Hex(std::span<const uint8_t> b) {
  return Hex(std::string_view{reinterpret_cast<const char*>(b.data()), b.size()});
}

std::string Bits(double d) {
  uint64_t u;
  std::memcpy(&u, &d, 8);
  return Hex(std::string_view{reinterpret_cast<const char*>(&u), 8});
}

std::string Bits(float f) {
  uint32_t u;
  std::memcpy(&u, &f, 4);
  return Hex(std::string_view{reinterpret_cast<const char*>(&u), 4});
}

template <typename T, typename F>
wpi::json List(std::span<const T> items, F f) {
  wpi::json out = wpi::json::array();
  for (const auto& i : items) {
    out.push_back(f(i));
  }
  return out;
}

wpi::json ValueJson(const nt::Value& v) {
  wpi::json out;
  switch (v.type()) {
    case NT_BOOLEAN:
      out = {{"t", "boolean"}, {"v", v.GetBoolean() ? "true" : "false"}};
      break;
    case NT_DOUBLE:
      out = {{"t", "double"}, {"v", Bits(v.GetDouble())}};
      break;
    case NT_INTEGER:
      out = {{"t", "int"}, {"v", std::to_string(v.GetInteger())}};
      break;
    case NT_FLOAT:
      out = {{"t", "float"}, {"v", Bits(v.GetFloat())}};
      break;
    case NT_STRING:
      out = {{"t", "string"}, {"v", Hex(v.GetString())}};
      break;
    case NT_RAW:
      out = {{"t", "raw"}, {"v", Hex(v.GetRaw())}};
      break;
    case NT_BOOLEAN_ARRAY:
      out = {{"t", "boolean[]"},
             {"v", List(v.GetBooleanArray(),
                        [](int b) { return b ? "true" : "false"; })}};
      break;
    case NT_DOUBLE_ARRAY:
      out = {{"t", "double[]"},
             {"v", List(v.GetDoubleArray(), [](double d) { return Bits(d); })}};
      break;
    case NT_INTEGER_ARRAY:
      out = {{"t", "int[]"},
             {"v", List(v.GetIntegerArray(),
                        [](int64_t i) { return std::to_string(i); })}};
      break;
    case NT_FLOAT_ARRAY:
      out = {{"t", "float[]"},
             {"v", List(v.GetFloatArray(), [](float f) { return Bits(f); })}};
      break;
    case NT_STRING_ARRAY:
      out = {{"t", "string[]"},
             {"v", List(v.GetStringArray(),
                        [](const std::string& s) { return Hex(s); })}};
      break;
    default:
      out = {{"t", "unassigned"}};
      break;
  }
  return out;
}

char* Finish(const wpi::json& j) {
  std::string s = j.dump(-1, ' ', false, wpi::json::error_handler_t::replace);
  char* out = static_cast<char*>(std::malloc(s.size() + 1));
  std::memcpy(out, s.c_str(), s.size() + 1);
  return out;
}

wpi::json OptionsJson(const nt::PubSubOptionsImpl& o) {
  return {{"periodic", Bits(o.periodic)},
          {"all", o.sendAll},
          {"topicsOnly", o.topicsOnly},
          {"prefix", o.prefixMatch}};
}

struct Recorder {
  wpi::json calls = wpi::json::array();
  wpi::json warnings = wpi::json::array();
  wpi::Logger logger;

  Recorder() {
    logger.SetLogger([this](unsigned int, const char*, unsigned int,
                            const char* msg) { warnings.push_back(msg); });
    logger.set_min_level(0);
  }
};

class ClientHandler : public nt::net::ClientMessageHandler {
 public:
  explicit ClientHandler(Recorder& r) : r{r} {}
  void ClientPublish(int pubuid, std::string_view name,
                     std::string_view typeStr, const wpi::json& properties,
                     const nt::PubSubOptionsImpl&) override {
    r.calls.push_back({{"m", "publish"},
                       {"pubuid", std::to_string(pubuid)},
                       {"name", Hex(name)},
                       {"type", Hex(typeStr)},
                       {"props", properties}});
  }
  void ClientUnpublish(int pubuid) override {
    r.calls.push_back({{"m", "unpublish"}, {"pubuid", std::to_string(pubuid)}});
  }
  void ClientSetProperties(std::string_view name,
                           const wpi::json& update) override {
    r.calls.push_back(
        {{"m", "setproperties"}, {"name", Hex(name)}, {"update", update}});
  }
  void ClientSubscribe(int subuid, std::span<const std::string> topicNames,
                       const nt::PubSubOptionsImpl& options) override {
    r.calls.push_back(
        {{"m", "subscribe"},
         {"subuid", std::to_string(subuid)},
         {"topics", List(topicNames, [](const std::string& s) { return Hex(s); })},
         {"options", OptionsJson(options)}});
  }
  void ClientUnsubscribe(int subuid) override {
    r.calls.push_back({{"m", "unsubscribe"}, {"subuid", std::to_string(subuid)}});
  }
  void ClientSetValue(int, const nt::Value&) override {}

 private:
  Recorder& r;
};

class ServerHandler : public nt::net::ServerMessageHandler {
 public:
  explicit ServerHandler(Recorder& r) : r{r} {}
  int ServerAnnounce(std::string_view name, int id, std::string_view typeStr,
                     const wpi::json& properties,
                     std::optional<int> pubuid) override {
    r.calls.push_back(
        {{"m", "announce"},
         {"name", Hex(name)},
         {"id", std::to_string(id)},
         {"type", Hex(typeStr)},
         {"pubuid", pubuid ? wpi::json(std::to_string(*pubuid)) : wpi::json()},
         {"props", properties}});
    return 0;
  }
  void ServerUnannounce(std::string_view name, int id) override {
    r.calls.push_back(
        {{"m", "unannounce"}, {"name", Hex(name)}, {"id", std::to_string(id)}});
  }
  void ServerPropertiesUpdate(std::string_view name, const wpi::json& update,
                              bool ack) override {
    r.calls.push_back({{"m", "properties"},
                       {"name", Hex(name)},
                       {"update", update},
                       {"ack", ack}});
  }
  void ServerSetValue(int, const nt::Value&) override {}

 private:
  Recorder& r;
};

}  // namespace

extern "C" {

char* nt4_oracle_binary(const uint8_t* data, size_t len) {
  std::span<const uint8_t> in{data, len};
  wpi::json msgs = wpi::json::array();
  wpi::json error;
  // As ntcore's ClientImpl and ServerImpl read a frame: until it is empty or
  // a message fails.
  while (!in.empty()) {
    int id = 0;
    nt::Value value;
    std::string err;
    try {
      if (!nt::net::WireDecodeBinary(&in, &id, &value, &err, 0)) {
        error = err;
        break;
      }
    } catch (const std::exception& e) {
      return Finish({{"msgs", msgs}, {"error", "exception"}, {"exception", e.what()}});
    }
    msgs.push_back({{"id", std::to_string(id)},
                    {"time", std::to_string(value.server_time())},
                    {"value", ValueJson(value)}});
  }
  return Finish({{"msgs", msgs}, {"error", error}});
}

char* nt4_oracle_client_text(const char* data, size_t len) {
  Recorder r;
  ClientHandler h{r};
  // ntcore lets some exceptions out (which would end its process); report
  // them instead.
  try {
    nt::net::WireDecodeText({data, len}, h, r.logger);
  } catch (const std::exception& e) {
    return Finish({{"calls", r.calls}, {"warnings", r.warnings}, {"exception", e.what()}});
  }
  return Finish({{"calls", r.calls}, {"warnings", r.warnings}});
}

char* nt4_oracle_server_text(const char* data, size_t len) {
  Recorder r;
  ServerHandler h{r};
  // ntcore lets some exceptions out (which would end its process); report
  // them instead.
  try {
    nt::net::WireDecodeText({data, len}, h, r.logger);
  } catch (const std::exception& e) {
    return Finish({{"calls", r.calls}, {"warnings", r.warnings}, {"exception", e.what()}});
  }
  return Finish({{"calls", r.calls}, {"warnings", r.warnings}});
}

void nt4_oracle_free(char* s) { std::free(s); }

}  // extern "C"

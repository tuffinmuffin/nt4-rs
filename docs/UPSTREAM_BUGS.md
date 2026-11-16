# Bugs found in ntcore / wpiutil

Side todo, tracked separately from the milestones: bugs nt4's ports and
interop tests found in allwpilib. nt4 doesn't copy any of them (see the
comments in the code). Line numbers are at allwpilib commit `4ccfe96`
unless noted. Earlier write-up of the two parser bugs:
[doc](https://claude.ai/artifact/QZfgEuNvsBzrsHrimL2qUG).

## 1. JSON parser: `null` after a repeated key gets the dropped value

- Where: `wpiutil/src/main/native/thirdparty/json/src/json.cpp`. The object
  case (line 1162) declares `json key, value;` once (line 1167) and reuses
  `value` for every member, clearing only `key` after each one.
- Repro: `{"a":1,"a":2,"b":null}` parses as `{"a":1,"b":2}`.
- Why: `emplace` of a repeated key doesn't take the value, so `value` still
  holds `2`; then the `null` case (line 993) returns success without
  assigning anything, and that stale `2` is stored for `b`.
- Fix: `value.clear()` next to `key.clear()` (or assign null in the `null`
  case).
- Found by: the nt4 fuzzer (see `crates/nt4-fuzz/README.md`).

## 2. `unsubscribe` with a bad `subuid` says "pubuid out of range"

- Where: `ntcore/src/main/native/cpp/net/WireDecoder.cpp` (the
  `"pubuid out of range"` strings at lines 165, 194, 308, 358; the
  unsubscribe one is a copy-paste of the publish check).
- Repro: `[{"method":"unsubscribe","params":{"subuid":2147483647}}]`.
- Fix: say "subuid out of range".

## 3. Server never sends the `setproperties` ack

- Where: `ntcore/src/main/native/cpp/server/ServerClient4Base.cpp:90`,
  `m_storage.SetProperties(nullptr, topic, update);`. With no client, the
  `ack: true` the spec asks for goes to nobody.
- Impact: low (ntcore clients ignore acks), but non-ntcore clients waiting
  for one never get it.
- nt4: acks the sender (`crates/nt4-core/src/server.rs`).

## 4. Stale "announced" flag after a topic is deleted

- Where: each client keeps `DenseMap<ServerTopic*, bool> m_announceSent`
  (`ntcore/src/main/native/cpp/server/ServerClient4Base.hpp:43`).
  `ServerStorage::DeleteTopic` (`ServerStorage.cpp:66`) sends unannounce to
  subscribers only, so a client that only published keeps the entry for the
  freed pointer.
- Impact: if a later topic is allocated at the same address, that client is
  treated as already announced and never gets its announce (nt4 hit the
  equivalent with reused topic ids).
- nt4: forgets the flag for every client on delete
  (`crates/nt4-core/src/server.rs`, near "ntcore keys \"announced\"").

## 5. Windows: persistent file mangles characters above U+FFFF

- Where: wpiutil's JSON writer decodes UTF-8 into `wint_t`, which is 16 bits
  on Windows.
- Repro: a persistent string containing 😀 is written as a lone ``
  instead of the surrogate pair `😀`. Linux and macOS are fine.
- Found by: `persistent_file_matches_ntcore_byte_for_byte` in
  `crates/nt4/tests/interop_server.rs` (the test leaves the emoji out on
  Windows).

## 6. `std::array<T, N>` struct schema only arrays the last field

- Where: `wpiutil/src/main/native/include/wpi/util/struct/Struct.hpp:379`,
  `MakeStructArraySchema` (used by `Struct<std::array<T, N>>` at line 491).
- Repro: for `T` with schema `int32 a; int32 b`, `std::array<T, 2>` gets
  schema `int32 a; int32 b[2]`, which describes 12 bytes, not 16.
- Fix: schema `T value[N]` (a nested struct array).
- nt4: has no fixed-array struct type; `StructTest StructFixedArray*` are in
  `scripts/upstream_na.txt`.

## Also worth mentioning (spec, not code)

- `ntcore/doc/networktables4.adoc`, data types table: the `protobuf` row
  says the data description is in a `protobuf` property. ntcore actually
  uses `proto:<message>` types with descriptors on `/.schema/proto:<file>`.
- The `/.schema/` convention (names, `structschema` type, retained) isn't
  written in any spec; it lives only in `NetworkTableInstance.hpp:955`
  (`AddSchema`) and `LocalStorageImpl.cpp:749`.

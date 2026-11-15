Three internal ntcore headers, copied unchanged from allwpilib tag v2026.2.2
(commit 7ca35e5678cf32caec6a1a866ca51d0136c4c398), so `../oracle.cpp` can call
ntcore's decoders in the pyntcore 2026.2.2 wheel:

- `net/WireDecoder.h` (ntcore/src/main/native/cpp/net/WireDecoder.h)
- `net/MessageHandler.h` (ntcore/src/main/native/cpp/net/MessageHandler.h)
- `PubSubOptions.h` (ntcore/src/main/native/cpp/PubSubOptions.h)

They are under WPILib's BSD license; see `LICENSE-WPILIB.md`. Update them with
the pyntcore version the oracle is built against.

Three internal ntcore headers, copied unchanged from allwpilib tag
v2027.0.0-alpha-8 (commit 4817543253e343afcf2b630537e995d74d21db62), so
`../oracle.cpp` can call ntcore's decoders in the pyntcore 2027.0.0a8 wheel:

- `net/WireDecoder.hpp` (ntcore/src/main/native/cpp/net/WireDecoder.hpp)
- `net/MessageHandler.hpp` (ntcore/src/main/native/cpp/net/MessageHandler.hpp)
- `PubSubOptions.hpp` (ntcore/src/main/native/cpp/PubSubOptions.hpp)

They are under WPILib's BSD license; see `LICENSE-WPILIB.md`. Update them with
the pyntcore version the oracle is built against.

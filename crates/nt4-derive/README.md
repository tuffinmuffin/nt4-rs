# nt4-derive

Unofficial: not affiliated with or endorsed by WPILib or FIRST.

`#[derive(NtStruct)]` for the [`nt4`](https://crates.io/crates/nt4) crate.
Turn on `nt4`'s `derive` feature and use it as `nt4::structs::NtStruct`;
this crate isn't meant to be used on its own.

```rust,ignore
use nt4::structs::NtStruct;

#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
struct ModuleState {
    velocity: f64,
    angle: Rotation2d, // another NtStruct
}
// struct:ModuleState, schema "double velocity;Rotation2d angle"
```

The schema and the nested schemas come from the fields, declared as
wpimath's C++ schemas declare them. Fieldless enums become
`enum {A=0,B=1} int8 variant`, as WPILib's Java `StructGenerator` writes
them. See the [docs](https://docs.rs/nt4-derive) for the options.

MIT licensed.

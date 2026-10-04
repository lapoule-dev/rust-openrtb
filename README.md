# rust-openrtb

OpenRTB for the hot path of bidders and SSPs, in Rust. Work in progress.

- **One model, two wire formats**: generated from the official IAB
  [`openrtb.proto`](https://github.com/InteractiveAdvertisingBureau/openrtb2.x/tree/main/proto)
  (OpenRTB 2.6 + Native 1.2). Protobuf via [buffa](https://github.com/anthropics/buffa)
  (owned, zero-copy and lazy views), and a generated OpenRTB JSON codec.
- **Built for real traffic**: lenient decoding (numbers as strings, `0`/`1`
  booleans, a lone value where an array is expected, `native.request` and `adm`
  as a string *or* an object, Native 1.0 `{"native":…}` wrapper).
- **Lossless**: unknown keys are kept and written back, also through protobuf;
  absent is never confused with `0`.
- **Complete AdCOM 1.0** (`adcom` crate): all 45 objects and 44 enumerated
  lists, generated from the specification itself and checked by a
  completeness test. Enums are typed and open (`EnumValue::Unknown(501)` is
  kept, never rejected); value names come from the spec definitions.

```rust
use openrtb_model::{OpenRtbJson, v2::BidRequest};

let req = BidRequest::from_json_slice(body)?;
let floor = req.imp[0].bidfloor();          // spec default applied (0.0)
let cur = req.imp[0].bidfloorcur();         // "USD" when absent
let json = req.to_json_vec();
```

## Layout

| Path | |
|---|---|
| `crates/openrtb-model` | OpenRTB 2.6: buffa types + generated JSON codec (`src/generated`, committed) |
| `crates/adcom` | AdCOM 1.0, generated from the spec via `proto/com/iabtechlab/adcom/v1/*.proto` |
| `crates/openrtb-json` | JSON runtime shared by the generated code |
| `crates/openrtb-codegen` | `cargo xtask codegen [--check]`: spec → proto → Rust for every crate |
| `crates/openrtb-bench` | criterion benchmarks |
| `fixtures/` | IAB 2.6 spec examples and scala-openrtb fixtures, used as golden tests |
| `reference/` | submodules: IAB specs, scala-openrtb, adcom-proto, iab-specs |

Clone with `git clone --recursive`. Regenerating requires `protoc` ≥ 27.

## Status (Apple M2, 1.5 KB BidSwitch request)

| | decode | encode |
|---|---|---|
| protobuf, lazy view | 0.31 µs | |
| protobuf, zero-copy view | 0.79 µs | |
| protobuf, owned | 1.39 µs | 0.36 µs |
| JSON, openrtb-model (serde_json / sonic-rs) | 3.67 / 3.16 µs | 1.05 µs |
| JSON, iab-specs 0.5.1 | 4.06 µs | 1.68 µs |

## License

BSD-3-Clause, see `LICENSE`. Third-party material (IAB spec and proto, scala-openrtb fixtures, iab-specs AdCOM lists) is Apache-2.0, see `NOTICE`.

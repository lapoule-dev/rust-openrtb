//! IAB AdCOM 1.0 — every object and enumerated list of the specification.
//!
//! Generated from `reference/AdCOM/AdCOM v1.0 FINAL.md`: the spec is parsed
//! into `proto/com/iabtechlab/adcom/v1/{adcom,enums}.proto`, compiled by buffa
//! (owned types, zero-copy and lazy views) and given an OpenRTB-style JSON
//! codec (integer enums, `0`/`1` booleans, lossless unknown keys).
//!
//! Enums are open: values the spec doesn't list (vendor-specific ≥ 500, newer
//! additions) are preserved as `EnumValue::Unknown`.
//!
//! Regenerate with `cargo xtask codegen`.

include!("generated/buffa/mod.rs");

#[allow(clippy::all, deprecated, unused_mut)]
mod adcom_gen {
    include!("generated/adcom.rs");
}

/// AdCOM enumerated lists (`DeviceType`, `ApiFramework`, …).
pub use com::iabtechlab::adcom::v1::enums;
/// AdCOM objects (`Ad`, `Placement`, `Device`, …).
pub use com::iabtechlab::adcom::v1::*;
pub use openrtb_json::OpenRtbJson;

//! Golden round trips on the IAB OpenRTB 2.6 spec examples (fixtures/iab) and
//! real-world fixtures from scala-openrtb (fixtures/scala, Apache-2.0):
//! JSON → model → JSON must be semantically equal to the input, and
//! JSON → model → protobuf → model → JSON must give the same JSON again.

use std::path::Path;

use buffa::Message;
use openrtb_model::OpenRtbJson;
use openrtb_model::v2::{BidRequest, BidResponse};
use serde_json::Value;

fn fixtures(prefix: &str) -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut out: Vec<_> = ["iab", "scala"]
        .iter()
        .flat_map(|dir| std::fs::read_dir(root.join(dir)).unwrap())
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_str().unwrap().contains(prefix))
        .map(|p| {
            (
                p.file_name().unwrap().to_str().unwrap().to_owned(),
                std::fs::read_to_string(&p).unwrap(),
            )
        })
        .collect();
    out.sort();
    assert!(!out.is_empty());
    out
}

/// Known, accepted differences: out-of-spec input normalized by the model.
const EXPECTED_DIFFS: &[(&str, &str)] = &[
    // `dnt` is a 0/1 boolean; BidSwitch sent 2, which reads as true → 1.
    ("bidswitch-bidrequest.json", "$.device.dnt"),
];

/// Numbers compare by value (1 == 1.0); `true`/`false` equal `1`/`0` (OpenRTB
/// booleans); an empty array equals an absent field (protobuf can't tell them apart).
fn same(a: &Value, b: &Value, path: &str, diffs: &mut Vec<String>) {
    match (a, b) {
        (Value::Array(x), Value::Null) | (Value::Null, Value::Array(x)) if x.is_empty() => {}
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))) {
                same(
                    x.get(k).unwrap_or(&Value::Null),
                    y.get(k).unwrap_or(&Value::Null),
                    &format!("{path}.{k}"),
                    diffs,
                );
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (u, v)) in x.iter().zip(y).enumerate() {
                same(u, v, &format!("{path}[{i}]"), diffs);
            }
        }
        (Value::Number(x), Value::Number(y)) if x.as_f64() == y.as_f64() => {}
        // Lenient typing: a number sent where the spec wants a string (`"ver": 1`).
        (Value::Number(n), Value::String(s)) | (Value::String(s), Value::Number(n))
            if n.to_string() == *s => {}
        (Value::Bool(x), Value::Number(n)) | (Value::Number(n), Value::Bool(x))
            if n.as_f64() == Some(*x as u8 as f64) => {}
        _ if a == b => {}
        _ => diffs.push(format!("{path}: {a} != {b}")),
    }
}

fn check<M: OpenRtbJson + serde::de::DeserializeOwned + Message>(name: &str, json: &str) {
    let model = M::from_json_str(json).unwrap_or_else(|e| panic!("{name}: decode: {e}"));
    let out = model.to_json_string();
    let mut diffs = Vec::new();
    let expected: Vec<_> = EXPECTED_DIFFS
        .iter()
        .filter(|(f, _)| *f == name)
        .map(|(_, p)| *p)
        .collect();
    same(
        &serde_json::from_str(json).unwrap(),
        &serde_json::from_str(&out).unwrap(),
        "$",
        &mut diffs,
    );
    diffs.retain(|d| !expected.iter().any(|p| d.starts_with(&format!("{p}:"))));
    assert!(
        diffs.is_empty(),
        "{name}: JSON round trip differs:\n{}\noutput: {out}",
        diffs.join("\n")
    );

    let via_proto = M::decode_from_slice(&model.encode_to_vec())
        .unwrap_or_else(|e| panic!("{name}: proto: {e}"));
    assert_eq!(
        via_proto.to_json_string(),
        out,
        "{name}: JSON → proto → JSON differs"
    );
}

#[test]
fn bid_requests() {
    for (name, json) in fixtures("bidrequest") {
        check::<BidRequest>(&name, &json);
    }
}

#[test]
fn bid_responses() {
    for (name, json) in fixtures("bidresponse") {
        check::<BidResponse>(&name, &json);
    }
}

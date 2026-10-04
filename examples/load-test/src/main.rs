//! Goose load test of the demo bidder.
//!
//! Users post randomized OpenRTB 2.6 bid requests to `/openrtb2/bid`, over
//! JSON (3/4 of the traffic) and protobuf (1/4). A request fails unless the
//! answer is a 204 no-bid, or a 200 whose body decodes as a `BidResponse`
//! answering that request.
//!
//! ```sh
//! cargo run --release -p load-test -- --host http://127.0.0.1:8080 \
//!     --users 64 --hatch-rate 64 --run-time 30s --report-file report.html
//! ```
//!
//! `LOADTEST_FIXTURES` points to the fixtures directory (default `fixtures`).

use std::path::PathBuf;
use std::sync::OnceLock;

use buffa::Message;
use goose::prelude::*;
use openrtb_model::OpenRtbJson;
use openrtb_model::v2::{BidRequest, BidResponse};

/// Pre-encoded requests: JSON and protobuf bytes, plus the request id.
struct Encoded {
    id: String,
    json: Vec<u8>,
    proto: Vec<u8>,
}

const POOL_SIZE: usize = 2_000;
const COUNTRIES: &[&str] = &[
    "USA", "FRA", "DEU", "GBR", "CAN", "ESP", "ITA", "BRA", "JPN", "IND",
];

static POOL: OnceLock<Vec<Encoded>> = OnceLock::new();

fn pool() -> &'static [Encoded] {
    POOL.get_or_init(|| {
        let dir =
            PathBuf::from(std::env::var("LOADTEST_FIXTURES").unwrap_or_else(|_| "fixtures".into()));
        let mut templates = Vec::new();
        for sub in ["iab", "scala"] {
            let Ok(entries) = std::fs::read_dir(dir.join(sub)) else {
                continue;
            };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.contains("bidrequest") {
                    let json = std::fs::read(e.path()).expect("read fixture");
                    templates.push(
                        BidRequest::from_json_slice(&json)
                            .unwrap_or_else(|err| panic!("{name}: {err}")),
                    );
                }
            }
        }
        assert!(
            !templates.is_empty(),
            "no bid request fixtures under {}",
            dir.display()
        );
        let mut rng = fastrand::Rng::with_seed(7);
        (0..POOL_SIZE)
            .map(|i| {
                let mut req = templates[i % templates.len()].clone();
                let id = format!("lt-{i}-{}", rng.u64(..));
                req.id = Some(id.clone());
                for imp in &mut req.imp {
                    imp.bidfloor = Some(imp.bidfloor() * (0.3 + rng.f64() * 2.7));
                }
                if rng.u8(..10) != 0 {
                    req.device
                        .get_or_insert_default()
                        .geo
                        .get_or_insert_default()
                        .country = Some(COUNTRIES[rng.usize(..COUNTRIES.len())].to_owned());
                }
                Encoded {
                    id,
                    json: req.to_json_vec(),
                    proto: req.encode_to_vec(),
                }
            })
            .collect()
    })
}

async fn bid(user: &mut GooseUser, protobuf: bool) -> TransactionResult {
    let req = &pool()[fastrand::usize(..POOL_SIZE)];
    let (content_type, body, name) = if protobuf {
        (
            "application/x-protobuf",
            req.proto.clone(),
            "bid (protobuf)",
        )
    } else {
        ("application/json", req.json.clone(), "bid (json)")
    };
    let builder = user
        .get_request_builder(&GooseMethod::Post, "/openrtb2/bid")?
        .header("content-type", content_type)
        .body(body);
    let goose_request = GooseRequest::builder()
        .name(name)
        .set_request_builder(builder)
        .build();
    let mut goose = user.request(goose_request).await?;

    let response = match goose.response {
        Ok(r) => r,
        Err(e) => {
            return user.set_failure(&format!("transport: {e}"), &mut goose.request, None, None);
        }
    };
    match response.status().as_u16() {
        204 => Ok(()),
        200 => {
            let headers = response.headers().clone();
            let bytes = match response.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    return user.set_failure(
                        &format!("body: {e}"),
                        &mut goose.request,
                        Some(&headers),
                        None,
                    );
                }
            };
            let decoded = if protobuf {
                BidResponse::decode_from_slice(&bytes).map_err(|e| e.to_string())
            } else {
                BidResponse::from_json_slice(&bytes).map_err(|e| e.to_string())
            };
            match decoded {
                Ok(r) if r.id.as_deref() == Some(req.id.as_str()) => Ok(()),
                Ok(_) => user.set_failure(
                    "response id does not match the request",
                    &mut goose.request,
                    Some(&headers),
                    None,
                ),
                Err(e) => user.set_failure(
                    &format!("invalid BidResponse: {e}"),
                    &mut goose.request,
                    Some(&headers),
                    None,
                ),
            }
        }
        status => user.set_failure(
            &format!("unexpected status {status}"),
            &mut goose.request,
            None,
            None,
        ),
    }
}

async fn bid_json(user: &mut GooseUser) -> TransactionResult {
    bid(user, false).await
}

async fn bid_protobuf(user: &mut GooseUser) -> TransactionResult {
    bid(user, true).await
}

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    // Build the request pool before the clock starts.
    let n = pool().len();
    eprintln!("load-test: {n} randomized requests ready");
    GooseAttack::initialize()?
        .register_scenario(
            scenario!("Exchange")
                .register_transaction(transaction!(bid_json).set_name("bid json").set_weight(3)?)
                .register_transaction(
                    transaction!(bid_protobuf)
                        .set_name("bid protobuf")
                        .set_weight(1)?,
                ),
        )
        .set_default(GooseDefault::Host, "http://127.0.0.1:8080")?
        .execute()
        .await?;
    Ok(())
}

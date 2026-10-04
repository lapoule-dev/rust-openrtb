//! HTTP layer: `POST /openrtb2/bid` (JSON or protobuf), win/bill notices, stats.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use buffa::{LazyMessageView, Message};
use openrtb_model::OpenRtbJson;
use openrtb_model::v2::{BidRequest, BidRequestLazyView};

use crate::bidder::{self, BidderConfig, to_usd};
use crate::campaigns::CampaignIndex;

pub const CT_JSON: &str = "application/json";
pub const CT_PROTOBUF: &str = "application/x-protobuf";

#[derive(Debug, Default)]
pub struct Stats {
    pub requests: AtomicU64,
    pub bids: AtomicU64,
    pub no_bids: AtomicU64,
    /// No-bids decided on the protobuf lazy view, without a full decode.
    pub fast_no_bids: AtomicU64,
    pub errors: AtomicU64,
    pub wins: AtomicU64,
    pub bills: AtomicU64,
    /// Sum of billed clearing prices, in micro-USD (CPM).
    pub spend_micros: AtomicU64,
}

pub struct AppState {
    pub index: CampaignIndex,
    pub cfg: BidderConfig,
    pub stats: Stats,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/openrtb2/bid", post(bid))
        .route("/win", get(win))
        .route("/bill", get(bill))
        .route("/stats", get(stats))
        .with_state(state)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Wire {
    Json,
    Protobuf,
}

fn wire(headers: &HeaderMap) -> Option<Wire> {
    let ct = match headers.get(header::CONTENT_TYPE) {
        None => return Some(Wire::Json),
        Some(v) => v.to_str().ok()?,
    };
    let mime = ct.split(';').next().unwrap_or("").trim();
    match mime {
        "application/json" | "" => Some(Wire::Json),
        "application/x-protobuf" | "application/protobuf" | "application/octet-stream" => {
            Some(Wire::Protobuf)
        }
        _ => None,
    }
}

fn no_bid(state: &AppState) -> Response {
    state.stats.no_bids.fetch_add(1, Relaxed);
    StatusCode::NO_CONTENT.into_response()
}

fn bad_request(state: &AppState, msg: String) -> Response {
    state.stats.errors.fetch_add(1, Relaxed);
    tracing::debug!(%msg, "bad bid request");
    (StatusCode::BAD_REQUEST, msg).into_response()
}

async fn bid(State(state): State<Arc<AppState>>, headers: HeaderMap, body: Bytes) -> Response {
    state.stats.requests.fetch_add(1, Relaxed);
    let Some(wire) = wire(&headers) else {
        state.stats.errors.fetch_add(1, Relaxed);
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    };

    let req = match wire {
        Wire::Json => match BidRequest::from_json_slice(&body) {
            Ok(r) => r,
            Err(e) => return bad_request(&state, format!("invalid JSON BidRequest: {e}")),
        },
        Wire::Protobuf => {
            // One scan over the top-level fields; imps/device decode on access.
            let lazy = match BidRequestLazyView::decode_lazy(&body) {
                Ok(v) => v,
                Err(e) => return bad_request(&state, format!("invalid protobuf BidRequest: {e}")),
            };
            if !bidder::may_bid(&state.index, &state.cfg, &lazy) {
                state.stats.fast_no_bids.fetch_add(1, Relaxed);
                return no_bid(&state);
            }
            match lazy.to_owned_message() {
                Ok(r) => r,
                Err(e) => return bad_request(&state, format!("invalid protobuf BidRequest: {e}")),
            }
        }
    };

    let Some(resp) = bidder::bid(&state.index, &state.cfg, &req) else {
        return no_bid(&state);
    };
    state.stats.bids.fetch_add(1, Relaxed);
    tracing::debug!(
        id = req.id.as_deref().unwrap_or(""),
        bids = resp.seatbid.iter().map(|s| s.bid.len()).sum::<usize>(),
        "bid"
    );
    let (ct, body) = match wire {
        Wire::Json => (CT_JSON, resp.to_json_vec()),
        Wire::Protobuf => (CT_PROTOBUF, resp.encode_to_vec()),
    };
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(ct)),
            (
                header::HeaderName::from_static("x-openrtb-version"),
                HeaderValue::from_static("2.6"),
            ),
        ],
        body,
    )
        .into_response()
}

/// Parses the clearing price the exchange substituted for `${AUCTION_PRICE}`.
fn clearing_price(q: &HashMap<String, String>) -> Result<(f64, String), String> {
    let raw = q.get("price").ok_or("missing price")?;
    let price: f64 = raw
        .parse()
        .map_err(|_| format!("price not substituted or invalid: {raw:?}"))?;
    let cur = q
        .get("cur")
        .filter(|c| !c.starts_with("${"))
        .map_or("USD", String::as_str);
    Ok((price, cur.to_owned()))
}

async fn win(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    match clearing_price(&q) {
        Ok((price, cur)) => {
            state.stats.wins.fetch_add(1, Relaxed);
            tracing::info!(
                auction = q.get("auction").map_or("", String::as_str),
                imp = q.get("imp").map_or("", String::as_str),
                crid = q.get("crid").map_or("", String::as_str),
                price,
                cur,
                "win"
            );
            StatusCode::OK.into_response()
        }
        Err(e) => {
            tracing::warn!(%e, "bad win notice");
            (StatusCode::BAD_REQUEST, e).into_response()
        }
    }
}

async fn bill(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    match clearing_price(&q) {
        Ok((price, cur)) => {
            state.stats.bills.fetch_add(1, Relaxed);
            let usd = to_usd(price, &cur).unwrap_or(price);
            state
                .stats
                .spend_micros
                .fetch_add((usd * 1e6).round() as u64, Relaxed);
            tracing::info!(
                auction = q.get("auction").map_or("", String::as_str),
                crid = q.get("crid").map_or("", String::as_str),
                price,
                cur,
                "bill"
            );
            StatusCode::OK.into_response()
        }
        Err(e) => {
            tracing::warn!(%e, "bad billing notice");
            (StatusCode::BAD_REQUEST, e).into_response()
        }
    }
}

async fn stats(State(state): State<Arc<AppState>>) -> axum::Json<serde_json::Value> {
    let s = &state.stats;
    let spend_cpm_usd = s.spend_micros.load(Relaxed) as f64 / 1e6;
    axum::Json(serde_json::json!({
        "requests": s.requests.load(Relaxed),
        "bids": s.bids.load(Relaxed),
        "no_bids": s.no_bids.load(Relaxed),
        "fast_no_bids": s.fast_no_bids.load(Relaxed),
        "errors": s.errors.load(Relaxed),
        "wins": s.wins.load(Relaxed),
        "bills": s.bills.load(Relaxed),
        // Prices are CPMs: one billed impression costs price / 1000.
        "spend_usd": spend_cpm_usd / 1000.0,
    }))
}

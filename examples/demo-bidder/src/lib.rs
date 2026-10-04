//! A demo OpenRTB 2.6 bidder built on `openrtb-model`.
//!
//! - [`campaigns`]: hardcoded native + banner campaigns and their targeting.
//! - [`bidder`]: request → response decision (pure, no I/O).
//! - [`server`]: axum routes (`POST /openrtb2/bid`, `GET /win`, `GET /bill`, `GET /stats`).

pub mod bidder;
pub mod campaigns;
pub mod server;

use std::sync::Arc;

pub use bidder::BidderConfig;
pub use server::{AppState, router};

/// Serves the bidder on `listener` until the future is dropped or fails.
pub async fn serve(listener: tokio::net::TcpListener, cfg: BidderConfig) -> std::io::Result<()> {
    let state = Arc::new(AppState {
        index: campaigns::CampaignIndex::demo(),
        cfg,
        stats: Default::default(),
    });
    axum::serve(listener, router(state)).await
}

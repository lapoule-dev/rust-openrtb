//! Starts the bidder on an ephemeral port and talks OpenRTB to it over HTTP,
//! in JSON and in protobuf.

use std::path::Path;

use buffa::Message;
use demo_bidder::BidderConfig;
use openrtb_model::OpenRtbJson;
use openrtb_model::v2::__buffa::oneof::bid_response::bid::AdmOneof;
use openrtb_model::v2::{BidRequest, BidResponse, NativeResponse};

async fn start() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let cfg = BidderConfig {
        public_url: base.clone(),
        ..Default::default()
    };
    tokio::spawn(demo_bidder::serve(listener, cfg));
    base
}

fn fixture(name: &str) -> BidRequest {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    BidRequest::from_json_slice(&std::fs::read(path).unwrap()).unwrap()
}

async fn post(base: &str, content_type: &str, body: Vec<u8>) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{base}/openrtb2/bid"))
        .header("content-type", content_type)
        .body(body)
        .send()
        .await
        .unwrap()
}

fn check_native_bid(req: &BidRequest, resp: &BidResponse) {
    assert_eq!(resp.id, req.id);
    assert_eq!(resp.cur.as_deref(), Some("USD"));
    let bid = &resp.seatbid[0].bid[0];
    let imp = &req.imp[0];
    assert_eq!(bid.impid, imp.id);
    assert!(bid.price.unwrap() >= imp.bidfloor(), "price under floor");
    assert!(bid.price.unwrap() > 0.0);
    assert!(bid.nurl.as_deref().unwrap().contains("${AUCTION_PRICE}"));
    assert!(bid.burl.as_deref().unwrap().contains("${AUCTION_PRICE}"));
    assert!(bid.crid.is_some() && bid.cid.is_some() && !bid.adomain.is_empty());
    // Native 1.2 response, JSON-encoded in adm.
    let Some(AdmOneof::Adm(adm)) = &bid.adm_oneof else {
        panic!("expected a string adm, got {:?}", bid.adm_oneof)
    };
    let native = NativeResponse::from_json_str(adm).unwrap();
    assert_eq!(native.ver.as_deref(), Some("1.2"));
    assert!(native.link.as_option().unwrap().url.is_some());
    assert!(!native.assets.is_empty());
}

#[tokio::test]
async fn native_bid_json_and_protobuf() {
    let base = start().await;
    let req = fixture("scala/openrtb-like-bidrequest-native.json");

    let res = post(&base, "application/json", req.to_json_vec()).await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["content-type"], "application/json");
    let resp = BidResponse::from_json_slice(&res.bytes().await.unwrap()).unwrap();
    check_native_bid(&req, &resp);

    let res = post(&base, "application/x-protobuf", req.encode_to_vec()).await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["content-type"], "application/x-protobuf");
    let resp = BidResponse::decode_from_slice(&res.bytes().await.unwrap()).unwrap();
    check_native_bid(&req, &resp);
}

#[tokio::test]
async fn no_bid_is_204() {
    let base = start().await;
    // Video only: no campaign can serve it.
    let video = fixture("iab/2.6-bidrequest-4-video.json");
    // Native, but the floor is above every campaign.
    let mut expensive = fixture("scala/bidswitch-bidrequest.json");
    expensive.imp[0].bidfloor = Some(50.0);

    for req in [&video, &expensive] {
        let res = post(&base, "application/json", req.to_json_vec()).await;
        assert_eq!(res.status(), 204);
        assert!(res.bytes().await.unwrap().is_empty());
        // The protobuf path decides on the lazy view.
        let res = post(&base, "application/octet-stream", req.encode_to_vec()).await;
        assert_eq!(res.status(), 204);
        assert!(res.bytes().await.unwrap().is_empty());
    }

    let stats: serde_json::Value = reqwest::get(format!("{base}/stats"))
        .await
        .unwrap()
        .text()
        .await
        .map(|t| serde_json::from_str(&t).unwrap())
        .unwrap();
    assert_eq!(stats["no_bids"], 4);
    assert_eq!(stats["fast_no_bids"], 2);
}

#[tokio::test]
async fn win_and_bill_notices() {
    let base = start().await;
    let get = |path: String| async move { reqwest::get(path).await.unwrap().status() };
    assert_eq!(get(format!("{base}/win?price=1.25&cur=USD")).await, 200);
    assert_eq!(get(format!("{base}/bill?price=0.92&cur=EUR")).await, 200);
    // Macro left unsubstituted by the exchange.
    assert_eq!(
        get(format!("{base}/win?price=${{AUCTION_PRICE}}")).await,
        400
    );

    let stats: serde_json::Value = serde_json::from_str(
        &reqwest::get(format!("{base}/stats"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(stats["wins"], 1);
    assert_eq!(stats["bills"], 1);
    // 0.92 EUR CPM = 1 USD CPM = 0.001 USD for one impression.
    assert!((stats["spend_usd"].as_f64().unwrap() - 0.001).abs() < 1e-9);
}

#[tokio::test]
async fn bad_requests() {
    let base = start().await;
    assert_eq!(
        post(&base, "application/json", b"{not json".to_vec())
            .await
            .status(),
        400
    );
    assert_eq!(
        post(&base, "application/x-protobuf", vec![0xff, 0xff])
            .await
            .status(),
        400
    );
    assert_eq!(
        post(&base, "text/plain", b"hello".to_vec()).await.status(),
        415
    );
}

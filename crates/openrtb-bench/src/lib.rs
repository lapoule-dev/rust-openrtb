//! Spike fixtures: the BidSwitch sample request from scala-openrtb, rebuilt as a buffa message.
use openrtb_model::com::iabtechlab::openrtb::v2::{
    __buffa::oneof::bid_request::native::RequestOneof, BidRequest, bid_request,
};

pub const BIDSWITCH_JSON: &str = include_str!("../../../fixtures/scala/bidswitch-bidrequest.json");

const NATIVE_REQUEST: &str = r#"{"plcmtcnt":1,"plcmttype":2,"privacy":1,"context":1,"contextsubtype":12,"assets":[{"id":1,"data":{"type":12},"required":1},{"title":{"len":50},"id":2,"required":1},{"id":3,"img":{"w":80,"h":80,"type":1},"required":1},{"id":4,"img":{"w":1200,"h":627,"type":3},"required":1},{"data":{"type":3},"id":5,"required":0},{"id":6,"data":{"len":100,"type":2},"required":1}],"ver":"1"}"#;

fn s(v: &str) -> Option<String> {
    Some(v.to_owned())
}

pub fn bidswitch_request() -> BidRequest {
    use bid_request::*;
    BidRequest {
        id: s("129ca6dd-5403-4476-a4a6-555d6a538bc4"),
        distributionchannel_oneof: App {
            id: s("pubnative_1009429"),
            publisher: Publisher {
                name: s(""),
                id: s("pubnative_1005292"),
                ..Default::default()
            }
            .into(),
            storeurl: s("https://play.google.com/store/apps/details?id=com.leo.appmaster"),
            bundle: s("com.leo.appmaster"),
            cat: vec!["IAB3".into()],
            name: s("PG_lock_pic"),
            ..Default::default()
        }
        .into(),
        wseat: vec!["167".into()],
        source: Source {
            fd: Some(false),
            ..Default::default()
        }
        .into(),
        user: User {
            id: s("793ff4b0-d077-4002-aeb6-b8ea64dd4b2b"),
            ..Default::default()
        }
        .into(),
        device: Device {
            connectiontype: Some(3),
            model: s("Micromax A096"),
            mccmnc: s("310-005"),
            language: s("en"),
            geo: Geo {
                country: s("IN"),
                lon: Some(85.1167),
                city: s("Patna"),
                lat: Some(25.6),
                zip: s("800002"),
                region: s("34"),
                r#type: Some(2),
                ..Default::default()
            }
            .into(),
            ifa: s("793ff4b0-d077-4002-aeb6-b8ea64dd4b2b"),
            osv: s("5.0.2"),
            os: s("Android"),
            carrier: s("Airtel"),
            devicetype: Some(1),
            ip: s("223.176.12.242"),
            ua: s("Dalvik/2.1.0 (Linux; U; Android 5.0.2; Micromax A096 Build/LRX21M)"),
            dnt: Some(true),
            ..Default::default()
        }
        .into(),
        tmax: Some(80),
        cur: vec!["USD".into()],
        imp: vec![Imp {
            bidfloor: Some(0.324),
            id: s("1"),
            native: Native {
                request_oneof: Some(RequestOneof::Request(NATIVE_REQUEST.to_owned())),
                ver: s("1.2"),
                ..Default::default()
            }
            .into(),
            exp: Some(1800),
            bidfloorcur: s("USD"),
            instl: Some(false),
            ..Default::default()
        }],
        bcat: ["IAB25-3", "BSW1", "BSW2", "BSW10", "BSW4", "IAB26"]
            .map(String::from)
            .to_vec(),
        at: Some(2),
        ..Default::default()
    }
}

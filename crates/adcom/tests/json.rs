use adcom::OpenRtbJson;
use adcom::enums::{
    ApiFramework, CategoryTaxonomy, ClickType, CreativeAttribute, DeviceType, OperatingSystem,
};
use adcom::{Ad, Device, Placement};
use buffa::{EnumValue, Message};

const AD: &str = r#"{
  "id": "555555",
  "adomain": ["ford.com"],
  "secure": 1,
  "attr": [1, 22, 999],
  "display": {
    "w": 300, "h": 250, "api": [3, 5],
    "banner": {"img": "https://cdn.example/creative.png", "link": {"url": "https://ford.com"}}
  },
  "audit": {"status": 3, "corr": {"note": "ok"}},
  "ext": {"vendor": {"x": [1, 2]}}
}"#;

#[test]
fn ad_round_trip_typed() {
    let ad = Ad::from_json_str(AD).unwrap();
    assert_eq!(ad.secure, Some(true));
    // Spec default (2) applied when absent.
    assert_eq!(
        ad.cattax(),
        EnumValue::Known(CategoryTaxonomy::ContentCategory20)
    );
    // 999 is not in AdCOM 1.0: kept, not rejected.
    assert_eq!(
        ad.attr,
        [
            EnumValue::Known(CreativeAttribute::AudioAdAutoplay),
            EnumValue::Known(CreativeAttribute::LimitedMotion),
            EnumValue::Unknown(999)
        ]
    );
    let display = ad.display.as_option().unwrap();
    assert_eq!(
        display.api,
        [
            EnumValue::Known(ApiFramework::Mraid10),
            EnumValue::Known(ApiFramework::Mraid20)
        ]
    );

    let json: serde_json::Value = serde_json::from_slice(&ad.to_json_vec()).unwrap();
    assert_eq!(json, serde_json::from_str::<serde_json::Value>(AD).unwrap());

    let via_proto = Ad::decode_from_slice(&ad.encode_to_vec()).unwrap();
    assert_eq!(via_proto.to_json_string(), ad.to_json_string());
}

#[test]
fn placement_and_device() {
    let p = Placement::from_json_str(
        r#"{"tagid":"t1","ssai":1,"display":{"w":300,"h":250,"clktype":2}}"#,
    )
    .unwrap();
    assert!(p.ssai());
    assert!(!p.reward());
    let d = p.display.as_option().unwrap();
    assert_eq!(
        d.clktype(),
        EnumValue::Known(ClickType::ClickableEmbeddedBrowserWebview)
    );
    assert!(!d.instl());

    let dev = Device::from_json_str(r#"{"type":4,"os":13,"osv":"17.1","dnt":0}"#).unwrap();
    assert_eq!(dev.r#type, Some(EnumValue::Known(DeviceType::Phone)));
    assert_eq!(dev.os, Some(EnumValue::Known(OperatingSystem::Ios)));
    assert_eq!(dev.dnt, Some(false));
}

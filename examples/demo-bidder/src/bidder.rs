//! Bidding logic: request → optional response. No I/O, so it is unit-testable
//! and the same for both wire formats.

use std::borrow::Cow;

use openrtb_model::OpenRtbJson;
use openrtb_model::v2::__buffa::oneof::bid_request::native::RequestOneof;
use openrtb_model::v2::__buffa::oneof::bid_response::bid::AdmOneof;
use openrtb_model::v2::__buffa::oneof::native_request::asset::AssetOneof as AssetReq;
use openrtb_model::v2::__buffa::oneof::native_response::asset::AssetOneof as AssetResp;
use openrtb_model::v2::bid_request::Imp;
use openrtb_model::v2::bid_response::{Bid, SeatBid};
use openrtb_model::v2::{
    BidRequest, BidRequestLazyView, BidResponse, NativeRequest, NativeResponse, native_response,
};

use crate::campaigns::{Campaign, CampaignIndex, Format, usd_rate};

/// AdCOM `NativeImageAssetType` / `NativeDataAssetType` values used below.
const IMG_ICON: i32 = 1;
const DATA_SPONSORED: i32 = 1;
const DATA_DESC: i32 = 2;
const DATA_RATING: i32 = 3;
const DATA_CTATEXT: i32 = 12;
/// AdCOM `CreativeType` (`bid.mtype`).
const MTYPE_BANNER: i32 = 1;
const MTYPE_NATIVE: i32 = 4;

#[derive(Debug, Clone)]
pub struct BidderConfig {
    /// Base URL of this bidder as seen by the exchange, for `nurl`/`burl`.
    pub public_url: String,
    /// Seat ID returned in `seatbid.seat`.
    pub seat: String,
    /// Requests whose `tmax` is below this many ms are not worth answering.
    pub min_tmax_ms: i32,
}

impl Default for BidderConfig {
    fn default() -> Self {
        Self {
            public_url: "http://127.0.0.1:8080".into(),
            seat: "demo-seat".into(),
            min_tmax_ms: 10,
        }
    }
}

/// Cheap pre-filter on a protobuf lazy view: only the request's own fields
/// and the few sub-messages it touches are decoded. `false` means a sure
/// no-bid; `true` means "decode and look closer". It must never reject a
/// request that [`bid`] would bid on.
pub fn may_bid(index: &CampaignIndex, cfg: &BidderConfig, req: &BidRequestLazyView<'_>) -> bool {
    if req.imp.is_empty() || req.tmax.is_some_and(|t| t < cfg.min_tmax_ms) {
        return false;
    }
    // Malformed deferred sub-messages decode as absent here; the full decode
    // that follows reports them.
    let device = req.device.get().ok().flatten();
    let devicetype = device.as_ref().and_then(|d| d.devicetype);
    let country = device
        .as_ref()
        .and_then(|d| d.geo.get().ok().flatten())
        .and_then(|g| g.country);
    // The full path falls back to user.geo.country, so only reject on a
    // device country when it is present.
    if country.is_some() && !index.reaches(country, devicetype) {
        return false;
    }
    req.imp.iter().flatten().any(|imp| {
        let has_format = !imp.banner.fragments().is_empty() || !imp.native.fragments().is_empty();
        let floor_usd = to_usd(
            imp.bidfloor.unwrap_or(0.0),
            imp.bidfloorcur.unwrap_or("USD"),
        );
        has_format && floor_usd.is_some_and(|f| f < index.max_cpm_usd)
    })
}

/// Decides on a full request. `None` = no bid.
pub fn bid(index: &CampaignIndex, cfg: &BidderConfig, req: &BidRequest) -> Option<BidResponse> {
    if req.imp.is_empty() || req.tmax.is_some_and(|t| t < cfg.min_tmax_ms) {
        return None;
    }
    // Bid in the first currency the exchange allows that we can price in.
    let cur = if req.cur.is_empty() {
        "USD"
    } else {
        req.cur
            .iter()
            .map(String::as_str)
            .find(|c| usd_rate(c).is_some())?
    };
    // `MessageField` derefs to a default instance when absent: no Option chains.
    let country = req
        .device
        .geo
        .country
        .as_deref()
        .or(req.user.geo.country.as_deref());
    let devicetype = req.device.devicetype;

    let bids: Vec<Bid> = req
        .imp
        .iter()
        .filter_map(|imp| {
            // We have no deals: skip private auctions.
            if imp.pmp.private_auction() {
                return None;
            }
            let floor_usd = to_usd(imp.bidfloor(), imp.bidfloorcur())?;
            let eligible = |c: &&Campaign| {
                c.max_cpm_usd > floor_usd
                    && c.targets_country(country)
                    && c.targets_device(devicetype)
                    && !c.blocked_by(&req.bcat, &req.badv)
            };
            let native = native_request(imp).and_then(|nreq| {
                index
                    .native()
                    .iter()
                    .filter(eligible)
                    .find_map(|c| Some((c, native_adm(c, &nreq)?)))
            });
            let banner = || {
                let b = imp.banner.as_option()?;
                let sizes = b.format.iter().map(|f| (f.w, f.h)).chain([(b.w, b.h)]);
                sizes
                    .filter_map(|(w, h)| Some((w?, h?)))
                    .flat_map(|(w, h)| index.banner(w, h).iter().filter(eligible))
                    .max_by(|a, b| a.max_cpm_usd.total_cmp(&b.max_cpm_usd))
                    .map(|c| (c, banner_adm(c)))
            };
            let (campaign, adm) = match native {
                Some(n) => Some(n),
                None => banner(),
            }?;
            Some(make_bid(cfg, imp, campaign, adm, floor_usd, cur))
        })
        .collect();

    if bids.is_empty() {
        return None;
    }
    Some(BidResponse {
        id: req.id.clone(),
        bidid: Some(format!("{:016x}", fastrand::u64(..))),
        cur: Some(cur.to_owned()),
        seatbid: vec![SeatBid {
            seat: Some(cfg.seat.clone()),
            bid: bids,
            ..Default::default()
        }],
        ..Default::default()
    })
}

fn make_bid(
    cfg: &BidderConfig,
    imp: &Imp,
    c: &Campaign,
    adm: String,
    floor_usd: f64,
    cur: &str,
) -> Bid {
    // Shade between the floor and the campaign max, never below the floor.
    let price_usd = floor_usd + (c.max_cpm_usd - floor_usd) * (0.4 + 0.6 * fastrand::f64());
    let rate = usd_rate(cur).unwrap_or(1.0);
    // Round up to 1/10000 so that rounding never drops under the floor.
    let price = (price_usd * rate * 10_000.0).ceil() / 10_000.0;
    let base = cfg.public_url.trim_end_matches('/');
    let q = format!(
        "auction=${{AUCTION_ID}}&bid=${{AUCTION_BID_ID}}&imp=${{AUCTION_IMP_ID}}\
         &seat=${{AUCTION_SEAT_ID}}&price=${{AUCTION_PRICE}}&cur=${{AUCTION_CURRENCY}}\
         &cid={}&crid={}",
        c.id, c.crid
    );
    let (w, h, mtype) = match c.format {
        Format::Banner { w, h } => (Some(w), Some(h), MTYPE_BANNER),
        Format::Native => (None, None, MTYPE_NATIVE),
    };
    Bid {
        id: Some(format!("{:08x}", fastrand::u32(..))),
        impid: imp.id.clone(),
        price: Some(price),
        nurl: Some(format!("{base}/win?{q}")),
        burl: Some(format!("{base}/bill?{q}")),
        adomain: vec![c.adomain.to_owned()],
        cid: Some(c.id.to_owned()),
        crid: Some(c.crid.to_owned()),
        cat: c.cat.iter().map(|s| (*s).to_owned()).collect(),
        w,
        h,
        mtype: Some(mtype),
        adm_oneof: Some(AdmOneof::Adm(adm)),
        ..Default::default()
    }
}

/// The imp's native request, whether it came as an object or as the
/// spec's JSON-encoded string.
fn native_request(imp: &Imp) -> Option<Cow<'_, NativeRequest>> {
    match imp.native.as_option()?.request_oneof.as_ref()? {
        RequestOneof::RequestNative(n) => Some(Cow::Borrowed(&**n)),
        RequestOneof::Request(s) => NativeRequest::from_json_str(s).ok().map(Cow::Owned),
    }
}

/// Builds the Native 1.2 response for `c`, as the JSON string that goes in
/// `adm`. `None` when a required asset can't be filled.
fn native_adm(c: &Campaign, req: &NativeRequest) -> Option<String> {
    let mut assets = Vec::with_capacity(req.assets.len());
    for a in &req.assets {
        let filled = match a.asset_oneof.as_ref() {
            Some(AssetReq::Title(t)) => Some(AssetResp::Title(Box::new(native_response::Title {
                text: Some(truncate(c.title, t.len).to_owned()),
                ..Default::default()
            }))),
            Some(AssetReq::Img(img)) => {
                let (dw, dh) = if img.r#type == Some(IMG_ICON) {
                    (80, 80)
                } else {
                    (1200, 627)
                };
                let w = img.w.or(img.wmin).unwrap_or(dw);
                let h = img.h.or(img.hmin).unwrap_or(dh);
                Some(AssetResp::Img(Box::new(native_response::Image {
                    url: Some(format!("{}-{w}x{h}.jpg", c.image_url)),
                    w: Some(w),
                    h: Some(h),
                    ..Default::default()
                })))
            }
            Some(AssetReq::Data(d)) => {
                let value = match d.r#type {
                    Some(DATA_SPONSORED) => Some(c.sponsor),
                    Some(DATA_DESC) => Some(c.description),
                    Some(DATA_RATING) => Some("4.5"),
                    Some(DATA_CTATEXT) => Some(c.cta),
                    _ => None,
                };
                value.map(|v| {
                    AssetResp::Data(Box::new(native_response::Data {
                        value: Some(truncate(v, d.len).to_owned()),
                        ..Default::default()
                    }))
                })
            }
            // No video creatives in this demo.
            Some(AssetReq::Video(_)) | None => None,
        };
        match filled {
            Some(asset) => assets.push(native_response::Asset {
                id: a.id,
                asset_oneof: Some(asset),
                ..Default::default()
            }),
            None if a.required == Some(true) => return None,
            None => {}
        }
    }
    let resp = NativeResponse {
        ver: Some("1.2".into()),
        assets,
        link: ::buffa::MessageField::some(native_response::Link {
            url: Some(c.click_url.to_owned()),
            ..Default::default()
        }),
        ..Default::default()
    };
    Some(resp.to_json_string())
}

fn banner_adm(c: &Campaign) -> String {
    let Format::Banner { w, h } = c.format else {
        unreachable!("banner campaign")
    };
    format!(
        r#"<a href="{}" target="_blank"><img src="{}" width="{w}" height="{h}" alt="{}"></a>"#,
        c.click_url, c.image_url, c.title
    )
}

/// Truncates to at most `len` characters (`len` absent or 0: no limit).
fn truncate(s: &str, len: Option<i32>) -> &str {
    match len {
        Some(n) if n > 0 => s.char_indices().nth(n as usize).map_or(s, |(i, _)| &s[..i]),
        _ => s,
    }
}

pub fn to_usd(amount: f64, cur: &str) -> Option<f64> {
    usd_rate(cur).map(|r| amount / r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_on_chars() {
        assert_eq!(truncate("Weekend 89€", Some(10)), "Weekend 89");
        assert_eq!(truncate("Weekend 89€", Some(11)), "Weekend 89€");
        assert_eq!(truncate("abc", None), "abc");
        assert_eq!(truncate("abc", Some(0)), "abc");
    }
}

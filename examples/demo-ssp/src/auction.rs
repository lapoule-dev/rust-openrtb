//! Response validation, the auction itself and macro substitution.

use openrtb_model::v2::bid_response::Bid;
use openrtb_model::v2::{BidRequest, BidResponse};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum AuctionType {
    /// The winner pays its bid.
    First,
    /// The winner pays max(second bid, floor) + 0.01, capped at its bid.
    Second,
}

/// One valid bid, with what the auction needs to rank and notify it.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub bidder: usize,
    pub bidid: String,
    pub seat: String,
    pub cur: String,
    pub price_usd: f64,
    pub bid: Bid,
}

#[derive(Debug, Clone)]
pub struct Win {
    pub candidate: Candidate,
    /// Clearing price in the winner's currency (CPM).
    pub price: f64,
    pub price_usd: f64,
}

/// Static FX rates (units of currency per USD), same table as the demo bidder.
pub fn usd_rate(cur: &str) -> Option<f64> {
    match cur {
        "USD" => Some(1.0),
        "EUR" => Some(0.92),
        "GBP" => Some(0.79),
        "INR" => Some(83.0),
        _ => None,
    }
}

fn floor_usd(req: &BidRequest, impid: &str) -> Option<f64> {
    let imp = req.imp.iter().find(|i| i.id.as_deref() == Some(impid))?;
    Some(imp.bidfloor() / usd_rate(imp.bidfloorcur())?)
}

/// Checks a response against its request; returns its valid bids and the
/// number of bids dropped (unknown imp, under the floor).
pub fn validate(
    req: &BidRequest,
    resp: &BidResponse,
    bidder: usize,
) -> Result<(Vec<Candidate>, usize), String> {
    if resp.id != req.id {
        return Err(format!(
            "response id {:?} != request id {:?}",
            resp.id, req.id
        ));
    }
    let cur = resp.cur.as_deref().unwrap_or("USD");
    if !req.cur.is_empty() && !req.cur.iter().any(|c| c == cur) {
        return Err(format!("currency {cur} not allowed"));
    }
    let rate = usd_rate(cur).ok_or_else(|| format!("unknown currency {cur}"))?;
    let mut ok = Vec::new();
    let mut dropped = 0;
    for seat in &resp.seatbid {
        for bid in &seat.bid {
            let impid = bid.impid.as_deref().unwrap_or("");
            let price = bid.price.unwrap_or(0.0);
            match floor_usd(req, impid) {
                Some(floor) if price > 0.0 && price / rate >= floor - 1e-9 => ok.push(Candidate {
                    bidder,
                    bidid: resp.bidid.clone().unwrap_or_default(),
                    seat: seat.seat.clone().unwrap_or_default(),
                    cur: cur.to_owned(),
                    price_usd: price / rate,
                    bid: bid.clone(),
                }),
                _ => dropped += 1,
            }
        }
    }
    Ok((ok, dropped))
}

/// Runs one auction per imp over all candidates.
pub fn run(req: &BidRequest, mut candidates: Vec<Candidate>, at: AuctionType) -> Vec<Win> {
    candidates.sort_by(|a, b| b.price_usd.total_cmp(&a.price_usd));
    let mut wins = Vec::new();
    for imp in &req.imp {
        let impid = imp.id.as_deref();
        let mut ranked = candidates
            .iter()
            .filter(|c| c.bid.impid.as_deref() == impid);
        let Some(winner) = ranked.next() else {
            continue;
        };
        let rate = usd_rate(&winner.cur).unwrap_or(1.0);
        let (price, price_usd) = match at {
            AuctionType::First => (winner.bid.price.unwrap_or(0.0), winner.price_usd),
            AuctionType::Second => {
                let floor = floor_usd(req, impid.unwrap_or("")).unwrap_or(0.0);
                let second = ranked.next().map_or(floor, |c| c.price_usd.max(floor));
                let usd = (second + 0.01).min(winner.price_usd);
                (((usd * rate) * 10_000.0).round() / 10_000.0, usd)
            }
        };
        wins.push(Win {
            candidate: winner.clone(),
            price,
            price_usd,
        });
    }
    wins
}

/// Substitutes the OpenRTB auction macros in a notice URL (or markup).
pub fn substitute(template: &str, req: &BidRequest, win: &Win) -> String {
    let c = &win.candidate;
    let price = format!("{:.4}", win.price);
    let macros = [
        ("${AUCTION_ID}", req.id.as_deref().unwrap_or("")),
        ("${AUCTION_BID_ID}", c.bidid.as_str()),
        ("${AUCTION_IMP_ID}", c.bid.impid.as_deref().unwrap_or("")),
        ("${AUCTION_SEAT_ID}", c.seat.as_str()),
        ("${AUCTION_AD_ID}", c.bid.adid.as_deref().unwrap_or("")),
        ("${AUCTION_PRICE}", price.as_str()),
        ("${AUCTION_CURRENCY}", c.cur.as_str()),
    ];
    let mut out = template.to_owned();
    for (k, v) in macros {
        if out.contains(k) {
            out = out.replace(k, v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use openrtb_model::v2::bid_request::Imp;

    fn req() -> BidRequest {
        BidRequest {
            id: Some("r1".into()),
            imp: vec![Imp {
                id: Some("1".into()),
                bidfloor: Some(0.5),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn cand(bidder: usize, price: f64) -> Candidate {
        Candidate {
            bidder,
            bidid: "b".into(),
            seat: "s".into(),
            cur: "USD".into(),
            price_usd: price,
            bid: Bid {
                impid: Some("1".into()),
                price: Some(price),
                nurl: Some("http://x/win?p=${AUCTION_PRICE}&a=${AUCTION_ID}".into()),
                ..Default::default()
            },
        }
    }

    #[test]
    fn first_and_second_price() {
        let r = req();
        let cands = vec![cand(0, 1.0), cand(1, 2.0)];
        let w = run(&r, cands.clone(), AuctionType::First);
        assert_eq!((w[0].candidate.bidder, w[0].price), (1, 2.0));
        let w = run(&r, cands, AuctionType::Second);
        assert_eq!((w[0].candidate.bidder, w[0].price), (1, 1.01));
        // Alone: pays floor + 0.01.
        let w = run(&r, vec![cand(0, 1.0)], AuctionType::Second);
        assert_eq!(w[0].price, 0.51);
        let url = substitute(w[0].candidate.bid.nurl.as_deref().unwrap(), &r, &w[0]);
        assert_eq!(url, "http://x/win?p=0.5100&a=r1");
    }
}

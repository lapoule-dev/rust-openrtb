//! Synthetic traffic: fixture bid requests with randomized ids, floors,
//! countries, currencies and tmax.

use std::path::Path;

use openrtb_model::OpenRtbJson;
use openrtb_model::v2::BidRequest;

/// ISO-3166-1 alpha-3, as the spec wants for `geo.country`.
const COUNTRIES: &[&str] = &[
    "USA", "FRA", "DEU", "GBR", "ESP", "IND", "BRA", "CAN", "JPN",
];
/// Used when a fixture has no `tmax`.
const DEFAULT_TMAX_MS: i32 = 120;

/// Loads every `*bidrequest*.json` under `<dir>/iab` and `<dir>/scala`.
pub fn load_templates(dir: &Path) -> std::io::Result<Vec<(String, BidRequest)>> {
    let mut out = Vec::new();
    for sub in ["iab", "scala"] {
        for entry in std::fs::read_dir(dir.join(sub))? {
            let path = entry?.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if !(name.contains("bidrequest") && name.ends_with(".json")) {
                continue;
            }
            let bytes = std::fs::read(&path)?;
            match BidRequest::from_json_slice(&bytes) {
                Ok(req) => out.push((format!("{sub}/{name}"), req)),
                Err(e) => tracing::warn!(file = %path.display(), %e, "skipping fixture"),
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// A fresh request from `template`: new ids, a random country (or none),
/// floors scaled ×0.3–×3, 15% of requests priced in EUR.
pub fn randomize(rng: &mut fastrand::Rng, template: &BidRequest, tmax: Option<i32>) -> BidRequest {
    let mut req = template.clone();
    req.id = Some(format!("{:016x}", rng.u64(..)));
    req.tmax = Some(tmax.or(template.tmax).unwrap_or(DEFAULT_TMAX_MS));
    req.source.get_or_insert_default().tid = Some(format!("{:032x}", rng.u128(..)));

    // Setting a deeply nested field: one `get_or_insert_default()` per level.
    let geo = req
        .device
        .get_or_insert_default()
        .geo
        .get_or_insert_default();
    geo.country = if rng.u8(..10) == 0 {
        None
    } else {
        Some(COUNTRIES[rng.usize(..COUNTRIES.len())].to_owned())
    };

    let eur = rng.u8(..100) < 15;
    if eur {
        req.cur = vec!["EUR".into()];
    }
    for imp in &mut req.imp {
        let base = imp.bidfloor().max(0.10);
        let floor = base * (0.3 + 2.7 * rng.f64());
        imp.bidfloor = Some((floor * 100.0).round() / 100.0);
        if eur {
            imp.bidfloorcur = Some("EUR".into());
        }
    }
    req
}

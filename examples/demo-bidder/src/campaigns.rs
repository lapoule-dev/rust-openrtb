//! A tiny in-memory campaign index: a handful of hardcoded native and banner
//! campaigns, with the targeting a real bidder would precompute.

use std::collections::HashMap;

/// What a campaign can serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A display banner of a fixed size.
    Banner { w: i32, h: i32 },
    /// A Native 1.2 ad (title, images, data assets).
    Native,
}

/// AdCOM device types (`device.devicetype`).
pub mod device_type {
    pub const MOBILE_TABLET: i32 = 1;
    pub const PC: i32 = 2;
    pub const CTV: i32 = 3;
    pub const PHONE: i32 = 4;
    pub const TABLET: i32 = 5;
}

#[derive(Debug, Clone)]
pub struct Campaign {
    pub id: &'static str,
    pub crid: &'static str,
    pub format: Format,
    /// ISO-3166-1 alpha-3 codes; empty = everywhere.
    pub countries: &'static [&'static str],
    /// AdCOM device types; empty = any device.
    pub device_types: &'static [i32],
    /// Highest CPM this campaign pays, in USD.
    pub max_cpm_usd: f64,
    pub adomain: &'static str,
    /// IAB content categories of the creative.
    pub cat: &'static [&'static str],
    pub title: &'static str,
    pub description: &'static str,
    pub sponsor: &'static str,
    pub cta: &'static str,
    pub click_url: &'static str,
    pub image_url: &'static str,
}

impl Campaign {
    pub fn targets_country(&self, country: Option<&str>) -> bool {
        if self.countries.is_empty() {
            return true;
        }
        match country.and_then(alpha3) {
            Some(c) => self.countries.contains(&c),
            None => false,
        }
    }

    pub fn targets_device(&self, devicetype: Option<i32>) -> bool {
        self.device_types.is_empty() || devicetype.is_some_and(|d| self.device_types.contains(&d))
    }

    /// `bcat` entries block a category and its subcategories (`IAB7` blocks `IAB7-39`).
    pub fn blocked_by(&self, bcat: &[impl AsRef<str>], badv: &[impl AsRef<str>]) -> bool {
        let cat_blocked = self.cat.iter().any(|c| {
            bcat.iter().any(|b| {
                let b = b.as_ref();
                *c == b || (c.starts_with(b) && c.as_bytes().get(b.len()) == Some(&b'-'))
            })
        });
        cat_blocked
            || badv
                .iter()
                .any(|d| d.as_ref().eq_ignore_ascii_case(self.adomain))
    }
}

/// Campaigns grouped by format, with the bounds used by the no-bid pre-filter.
#[derive(Debug)]
pub struct CampaignIndex {
    native: Vec<Campaign>,
    banner: HashMap<(i32, i32), Vec<Campaign>>,
    /// Highest `max_cpm_usd` of all campaigns: a floor above it is a sure no-bid.
    pub max_cpm_usd: f64,
}

impl CampaignIndex {
    pub fn new(campaigns: Vec<Campaign>) -> Self {
        let max_cpm_usd = campaigns.iter().map(|c| c.max_cpm_usd).fold(0.0, f64::max);
        let mut native = Vec::new();
        let mut banner: HashMap<_, Vec<_>> = HashMap::new();
        for c in campaigns {
            match c.format {
                Format::Native => native.push(c),
                Format::Banner { w, h } => banner.entry((w, h)).or_default().push(c),
            }
        }
        // Highest bidder first: the first eligible campaign wins.
        let by_cpm = |a: &Campaign, b: &Campaign| b.max_cpm_usd.total_cmp(&a.max_cpm_usd);
        native.sort_by(by_cpm);
        banner.values_mut().for_each(|v| v.sort_by(by_cpm));
        Self {
            native,
            banner,
            max_cpm_usd,
        }
    }

    pub fn native(&self) -> &[Campaign] {
        &self.native
    }

    pub fn banner(&self, w: i32, h: i32) -> &[Campaign] {
        self.banner.get(&(w, h)).map_or(&[], Vec::as_slice)
    }

    /// Can any campaign serve this country / device type at all?
    pub fn reaches(&self, country: Option<&str>, devicetype: Option<i32>) -> bool {
        self.native
            .iter()
            .chain(self.banner.values().flatten())
            .any(|c| c.targets_country(country) && c.targets_device(devicetype))
    }

    /// The demo campaigns.
    pub fn demo() -> Self {
        use device_type::*;
        let base = Campaign {
            id: "",
            crid: "",
            format: Format::Native,
            countries: &[],
            device_types: &[],
            max_cpm_usd: 1.0,
            adomain: "",
            cat: &[],
            title: "",
            description: "",
            sponsor: "",
            cta: "Learn more",
            click_url: "",
            image_url: "",
        };
        Self::new(vec![
            Campaign {
                id: "cmp-travel",
                crid: "cr-travel-native-1",
                countries: &["FRA", "DEU", "GBR", "ESP", "ITA"],
                max_cpm_usd: 2.40,
                adomain: "voyages.example",
                cat: &["IAB20-18"],
                title: "Weekend in Lisbon from 89€",
                description: "Flights and hotels bundled, free cancellation up to 48h before departure.",
                sponsor: "Voyages Example",
                cta: "Book now",
                click_url: "https://voyages.example/lisbon",
                image_url: "https://cdn.voyages.example/lisbon",
                ..base.clone()
            },
            Campaign {
                id: "cmp-mobile-game",
                crid: "cr-game-native-7",
                countries: &["IND", "USA", "BRA"],
                device_types: &[MOBILE_TABLET, PHONE, TABLET],
                max_cpm_usd: 1.10,
                adomain: "game.example",
                cat: &["IAB9-30"],
                title: "Build your kingdom",
                description: "The strategy game played by 10M people. Free to play.",
                sponsor: "Game Studio Example",
                cta: "Install",
                click_url: "https://game.example/install",
                image_url: "https://cdn.game.example/kingdom",
                ..base.clone()
            },
            Campaign {
                id: "cmp-news",
                crid: "cr-news-native-2",
                max_cpm_usd: 0.60,
                adomain: "news.example",
                cat: &["IAB12"],
                title: "The morning briefing",
                description: "Everything you need to know today, in five minutes.",
                sponsor: "News Example",
                cta: "Read",
                click_url: "https://news.example/briefing",
                image_url: "https://cdn.news.example/briefing",
                ..base.clone()
            },
            Campaign {
                id: "cmp-cars",
                crid: "cr-cars-mrec-3",
                format: Format::Banner { w: 300, h: 250 },
                max_cpm_usd: 1.80,
                adomain: "cars.example",
                cat: &["IAB2"],
                title: "The new electric hatchback",
                click_url: "https://cars.example/ev",
                image_url: "https://cdn.cars.example/ev-300x250.png",
                ..base.clone()
            },
            Campaign {
                id: "cmp-bank",
                crid: "cr-bank-leader-1",
                format: Format::Banner { w: 728, h: 90 },
                countries: &["USA", "GBR", "CAN"],
                device_types: &[MOBILE_TABLET, PC, PHONE, TABLET],
                max_cpm_usd: 3.20,
                adomain: "bank.example",
                cat: &["IAB13-7"],
                title: "Zero-fee checking",
                click_url: "https://bank.example/checking",
                image_url: "https://cdn.bank.example/checking-728x90.png",
                ..base.clone()
            },
            Campaign {
                id: "cmp-shoes",
                crid: "cr-shoes-sky-4",
                format: Format::Banner { w: 300, h: 600 },
                device_types: &[PC],
                max_cpm_usd: 2.10,
                adomain: "shoes.example",
                cat: &["IAB18-3"],
                title: "Running shoes -30%",
                click_url: "https://shoes.example/sale",
                image_url: "https://cdn.shoes.example/sale-300x600.png",
                ..base
            },
        ])
    }
}

/// Normalizes a country code to ISO-3166-1 alpha-3 (the OpenRTB spec value);
/// a few alpha-2 codes are accepted since real traffic sends them too.
pub fn alpha3(country: &str) -> Option<&'static str> {
    const CODES: &[(&str, &str)] = &[
        ("US", "USA"),
        ("FR", "FRA"),
        ("DE", "DEU"),
        ("GB", "GBR"),
        ("ES", "ESP"),
        ("IT", "ITA"),
        ("IN", "IND"),
        ("BR", "BRA"),
        ("CA", "CAN"),
        ("JP", "JPN"),
    ];
    CODES
        .iter()
        .find(|(a2, a3)| country.eq_ignore_ascii_case(a2) || country.eq_ignore_ascii_case(a3))
        .map(|(_, a3)| *a3)
}

/// Static FX rates (units of currency per USD). A real bidder would refresh them.
pub fn usd_rate(cur: &str) -> Option<f64> {
    match cur {
        "USD" => Some(1.0),
        "EUR" => Some(0.92),
        "GBP" => Some(0.79),
        "INR" => Some(83.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bcat_blocks_subcategories() {
        let idx = CampaignIndex::demo();
        let shoes = idx.banner(300, 600).first().unwrap();
        assert!(shoes.blocked_by(&["IAB18"], &[] as &[&str]));
        assert!(shoes.blocked_by(&["IAB18-3"], &[] as &[&str]));
        assert!(!shoes.blocked_by(&["IAB1"], &[] as &[&str]));
        assert!(shoes.blocked_by(&[] as &[&str], &["SHOES.example"]));
    }

    #[test]
    fn countries() {
        let idx = CampaignIndex::demo();
        let travel = &idx.native()[0];
        assert_eq!(travel.id, "cmp-travel");
        assert!(travel.targets_country(Some("FRA")));
        assert!(travel.targets_country(Some("fr")));
        assert!(!travel.targets_country(Some("USA")));
        assert!(!travel.targets_country(None));
    }
}

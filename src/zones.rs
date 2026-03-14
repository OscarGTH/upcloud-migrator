/// All available UpCloud zones.
pub struct ZoneInfo {
    pub slug: &'static str,
    pub city: &'static str,
    pub region: &'static str,
}

pub const ZONES: &[ZoneInfo] = &[
    // Europe
    ZoneInfo { slug: "nl-ams1", city: "Amsterdam",  region: "Europe" },
    ZoneInfo { slug: "dk-cph1", city: "Copenhagen", region: "Europe" },
    ZoneInfo { slug: "de-fra1", city: "Frankfurt",  region: "Europe" },
    ZoneInfo { slug: "fi-hel1", city: "Helsinki",   region: "Europe" },
    ZoneInfo { slug: "fi-hel2", city: "Helsinki 2", region: "Europe" },
    ZoneInfo { slug: "uk-lon1", city: "London",     region: "Europe" },
    ZoneInfo { slug: "es-mad1", city: "Madrid",     region: "Europe" },
    ZoneInfo { slug: "no-svg1", city: "Stavanger",  region: "Europe" },
    ZoneInfo { slug: "se-sto1", city: "Stockholm",  region: "Europe" },
    ZoneInfo { slug: "pl-waw1", city: "Warsaw",     region: "Europe" },
    // Americas
    ZoneInfo { slug: "us-chi1", city: "Chicago",    region: "Americas" },
    ZoneInfo { slug: "us-nyc1", city: "New York",   region: "Americas" },
    ZoneInfo { slug: "us-sjo1", city: "San Jose",   region: "Americas" },
    // Asia-Pacific
    ZoneInfo { slug: "sg-sin1", city: "Singapore",  region: "Asia-Pacific" },
    ZoneInfo { slug: "au-syd1", city: "Sydney",     region: "Asia-Pacific" },
];

/// Return the index of `slug` in ZONES, defaulting to de-fra1 (index 2).
pub fn find_zone_idx(slug: &str) -> usize {
    ZONES.iter().position(|z| z.slug == slug).unwrap_or(2)
}

/// Map a zone slug to the nearest UpCloud Object Storage region.
pub fn zone_to_objstorage_region(slug: &str) -> &'static str {
    match slug {
        "de-fra1" | "nl-ams1" => "europe-1",
        "fi-hel1" | "fi-hel2" | "se-sto1" | "dk-cph1" | "no-svg1" => "europe-2",
        "pl-waw1" | "uk-lon1" | "es-mad1" => "europe-3",
        "us-nyc1" | "us-chi1" | "us-sjo1" => "us-east-1",
        "sg-sin1" | "au-syd1" => "asia-1",
        _ => "europe-1",
    }
}

/// Number of visual list rows before zone index `idx` (accounts for region headers).
/// Region headers appear once before each new region group.
pub fn zone_idx_to_visual_row(idx: usize) -> usize {
    // Europe header at row 0, zones 0–9 at rows 1–10
    // Americas header at row 11, zones 10–12 at rows 12–14
    // Asia-Pacific header at row 15, zones 13–14 at rows 16–17
    if idx < 10 {
        idx + 1
    } else if idx < 13 {
        idx + 2
    } else {
        idx + 3
    }
}

/// Total visual rows in the zone list (zones + region headers).
pub const ZONE_LIST_VISUAL_ROWS: usize = ZONES.len() + 3; // 15 zones + 3 headers

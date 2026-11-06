//! How `/compute` draws a Listing: its card, its price, what it sells and
//! whether its Provider is live. Amounts stay in µUSDC and stay exact; only
//! their grouping and the units of time and size are made readable.

use std::collections::HashMap;

use nostr::key::PublicKey;
use relay_ui::chrome::{self, Apps, escape};

use crate::directory::{Listing, PerHour};
use crate::provider::{listing_path, provider_link};

/// `n` with its thousands grouped by commas: `1,000,000`.
pub(crate) fn group(n: u128) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A Lease Interval of `seconds` in the largest unit it is a whole number
/// of: `1 h`, `10 min`, `7 s`, `2 days`.
pub(crate) fn interval(seconds: u64) -> String {
    match seconds {
        s if s > 0 && s.is_multiple_of(86_400) => match s / 86_400 {
            1 => "1 day".to_owned(),
            days => format!("{days} days"),
        },
        s if s > 0 && s.is_multiple_of(3_600) => format!("{} h", s / 3_600),
        s if s > 0 && s.is_multiple_of(60) => format!("{} min", s / 60),
        s => format!("{s} s"),
    }
}

/// An hour's price, exactly: `514,285 + 5/7 µUSDC`.
pub(crate) fn hourly(hour: PerHour) -> String {
    match hour.remainder {
        0 => format!("{} µUSDC", group(hour.whole)),
        r => format!("{} + {}/{} µUSDC", group(hour.whole), r, hour.interval),
    }
}

/// What a Listing sells, as `(what, how much)` in a reader's units: CPU,
/// memory, storage and GPU first, then anything else under its own name.
pub(crate) fn spec_items(resources: &[(String, String)]) -> Vec<(String, String)> {
    let number = |amount: &str| amount.trim().parse::<u64>().ok();
    let mut known: Vec<(u8, String, String)> = Vec::new();
    for (what, amount) in resources {
        let item = match (what.to_lowercase().as_str(), number(amount)) {
            ("cpu_millicores" | "millicores", Some(m)) => (0, "CPU", millicores(m)),
            ("vcpu" | "vcpus" | "cpu" | "cpus", Some(n)) => (0, "CPU", format!("{n} vCPU")),
            ("memory_mb" | "memory_mib" | "ram_mb", Some(mb)) => (1, "Memory", megabytes(mb)),
            ("storage_gb" | "disk_gb", Some(gb)) => (2, "Storage", format!("{gb} GB")),
            ("memory" | "ram", _) => (1, "Memory", amount.clone()),
            ("storage" | "disk", _) => (2, "Storage", amount.clone()),
            ("gpu" | "gpus", _) => (3, "GPU", amount.clone()),
            ("", _) => (4, "", amount.clone()),
            (other, _) => {
                let mut label = other.replace('_', " ");
                if let Some(first) = label.get_mut(..1) {
                    first.make_ascii_uppercase();
                }
                known.push((4, label, amount.clone()));
                continue;
            }
        };
        known.push((item.0, item.1.to_owned(), item.2));
    }
    known.sort_by_key(|(order, _, _)| *order);
    known
        .into_iter()
        .map(|(_, what, amount)| (what, amount))
        .collect()
}

/// `500` millicores as `0.5 vCPU`, exactly.
fn millicores(m: u64) -> String {
    let (whole, part) = (m / 1_000, m % 1_000);
    match part {
        0 => format!("{whole} vCPU"),
        part => {
            let fraction = format!("{part:03}");
            format!("{whole}.{} vCPU", fraction.trim_end_matches('0'))
        }
    }
}

/// `4096` MB as `4 GB`; anything not a whole number of GB stays in MB.
fn megabytes(mb: u64) -> String {
    match mb {
        mb if mb >= 1_024 && mb.is_multiple_of(1_024) => format!("{} GB", mb / 1_024),
        mb => format!("{mb} MB"),
    }
}

/// [`spec_items`] as one line of text, for a card or a description.
pub(crate) fn spec_text(listing: &Listing) -> String {
    spec_items(&listing.resources)
        .into_iter()
        .map(|(what, amount)| match what.as_str() {
            "CPU" | "" => amount,
            "GPU" => format!("GPU {amount}"),
            "Storage" => format!("{amount} storage"),
            _ => amount,
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The Listing's resources as a grid of tiles.
pub(crate) fn spec_html(listing: &Listing, large: bool) -> String {
    let items = spec_items(&listing.resources);
    if items.is_empty() {
        return String::new();
    }
    let tiles: String = items
        .iter()
        .map(|(what, amount)| {
            format!(
                "<div><dt>{}</dt><dd>{}</dd></div>",
                match what.is_empty() {
                    true => "Resources".to_owned(),
                    false => escape(what),
                },
                escape(amount)
            )
        })
        .collect();
    format!(
        "<dl class=\"spec{}\">{tiles}</dl>",
        if large { " spec-lg" } else { "" }
    )
}

/// Live with its capacity, or down.
pub(crate) fn status_chip(listing: &Listing) -> String {
    match (listing.live, listing.available) {
        (true, Some(0)) => {
            "<span class=\"live-chip full\" data-status=\"live\">Live · none available</span>"
                .to_owned()
        }
        (true, Some(n)) => {
            format!(
                "<span class=\"live-chip live\" data-status=\"live\">Live · {n} available</span>"
            )
        }
        (true, None) => {
            "<span class=\"live-chip live\" data-status=\"live\">Live</span>".to_owned()
        }
        (false, _) => "<span class=\"live-chip\" data-status=\"down\">Down</span>".to_owned(),
    }
}

/// A Provider's own status: live, and since when its Liveness was signed.
pub(crate) fn provider_status(live_since: Option<u64>) -> String {
    match live_since {
        Some(at) => format!(
            "<span class=\"live-chip live\" data-status=\"live\">Live</span>\
             <span data-live-since=\"{at}\">Liveness signed {}</span>",
            chrome::ago(nostr::types::Timestamp::from_secs(at)),
        ),
        None => "<span class=\"live-chip\" data-status=\"down\">Down</span>\
                 <span>No current Liveness</span>"
            .to_owned(),
    }
}

/// The price per hour, large, over the exact price per Lease Interval.
pub(crate) fn price_tag(listing: &Listing) -> String {
    let Some(price) = listing.price else {
        return "<div class=\"price-tag none\"><span class=\"hourly\">No price stated</span></div>"
            .to_owned();
    };
    let exact = match listing.lease_interval_s {
        Some(s) => format!(
            "{} µUSDC per {} Lease Interval",
            group(price.into()),
            interval(s)
        ),
        None => format!("{} µUSDC per Lease Interval", group(price.into())),
    };
    match listing.per_hour() {
        Some(hour) => format!(
            "<div class=\"price-tag\" data-price-hour=\"{}\"><span class=\"hourly{}\">{} \
             <span class=\"unit\">per hour</span></span><span class=\"exact\">{exact}</span></div>",
            hour.whole,
            if hour.remainder == 0 { "" } else { " fraction" },
            escape(&hourly(hour)),
        ),
        None => format!(
            "<div class=\"price-tag\"><span class=\"hourly\">{} µUSDC</span>\
             <span class=\"exact\">{exact}</span></div>",
            group(price.into())
        ),
    }
}

/// What a Warm Standby costs, or that the Listing sells none.
pub(crate) fn standby_text(listing: &Listing) -> String {
    match (listing.standby_price, listing.lease_interval_s) {
        (Some(price), Some(s)) => {
            format!("Standby {} µUSDC per {}", group(price.into()), interval(s))
        }
        (Some(price), None) => format!("Standby {} µUSDC", group(price.into())),
        (None, _) => "No standbys".to_owned(),
    }
}

/// The Listing's arch, isolation and capabilities, as chips.
pub(crate) fn tag_chips(listing: &Listing) -> String {
    let mut chips: Vec<String> = Vec::new();
    if let Some(arch) = &listing.arch {
        chips.push(format!(
            "<span class=\"chip\" title=\"Arch\">{}</span>",
            escape(arch)
        ));
    }
    if let Some(isolation) = &listing.isolation {
        chips.push(format!(
            "<span class=\"chip\" title=\"Isolation\">{}</span>",
            escape(isolation)
        ));
    }
    for cap in &listing.capabilities {
        chips.push(format!(
            "<span class=\"chip chip-topic\" title=\"Capability\">{}</span>",
            escape(cap)
        ));
    }
    match chips.is_empty() {
        true => String::new(),
        false => format!("<div class=\"chips\">{}</div>", chips.concat()),
    }
}

/// The Anyone network's logo, drawn inline like the other icons (the page
/// policy loads no image from elsewhere). A macro so `concat!` can take it.
macro_rules! anyone_mark {
    () => {
        "<svg class=\"anyone-mark\" viewBox=\"0 0 483 483\" aria-hidden=\"true\">\
     <path class=\"anyone-band\" d=\"M478 288C474 311 466 332 456 352C446 371 434 388 420 404C376 452 312 483 241 483C170 483 107 452 63 404C49 388 37 371 27 352C17 334 10 314 6 294C21 319 49 335 81 335C84 335 87 335 90 335C92 338 94 340 95 343C128 389 181 419 242 419C302 419 355 389 388 344C389 341 391 338 393 335C395 335 397 335 400 335C434 335 463 316 478 288C478 288 478 288 478 288z\"/>\
     <path class=\"anyone-ring\" d=\"M416 242C415 253 406 261 394 261C390 261 385 260 382 258C381 257 381 257 380 256C362 243 342 232 320 224C295 215 269 210 241 210C214 210 187 215 163 224C141 232 121 243 102 256C102 257 101 257 101 258C97 260 93 261 89 261C77 261 67 252 67 240C67 240 67 240 67 239C67 239 67 238 67 238C67 210 73 183 85 160C104 121 138 91 178 75C178 75 178 75 178 75C198 67 219 63 241 63C252 63 263 64 273 66C328 76 374 112 398 160C410 183 416 210 416 238C416 238 416 239 416 240C416 240 416 241 416 242C416 242 416 242 416 242C416 242 416 242 416 242zM241 0C108 0 0 108 0 241C0 242 0 242 0 242C0 243 0 243 0 244C1 276 19 303 46 317C57 322 69 325 83 325C83 325 84 325 84 325C86 325 88 325 90 325C104 324 117 319 128 311C130 310 132 309 134 307C164 286 201 273 241 273C281 273 318 286 349 307C351 309 352 310 354 311C365 319 379 324 393 325C395 325 397 325 399 325C399 325 400 325 400 325C413 325 426 322 437 317C463 303 482 276 483 244C483 243 483 243 483 242C483 242 483 242 483 241C483 108 374 0 241 0C241 0 241 0 241 0C241 0 241 0 241 0z\"/></svg>"
    };
}

/// The Anyone-only mark of a Hidden Provider, with the Anyone logo.
pub(crate) const ANYONE_ONLY: &str = concat!(
    "<span class=\"chip chip-hidden\" data-hidden=\"true\" title=\"Reachable only through \
     Anyone; it publishes no host\">",
    anyone_mark!(),
    "Anyone-only</span>"
);

/// One Listing as a card; `with_provider` names its Provider on it.
pub(crate) fn listing_card(
    apps: &Apps,
    names: &HashMap<PublicKey, String>,
    listing: &Listing,
    with_provider: bool,
) -> String {
    let by = match with_provider {
        true => format!(
            "<div class=\"by\">{}{}{}</div>",
            chrome::person_avatar(apps, &listing.provider, true),
            provider_link(names, &listing.provider),
            if listing.hidden { ANYONE_ONLY } else { "" },
        ),
        false => String::new(),
    };
    format!(
        "<article class=\"listing-card\" data-listing=\"{name}\" data-status=\"{status}\">\
         <div class=\"top\"><h3><a href=\"{path}\">{name}</a></h3>{chip}</div>{by}\
         {price}{spec}{tags}<div class=\"foot\"><span>{standby}</span><a href=\"{path}\">Details</a></div>\
         </article>",
        name = escape(&listing.name),
        status = if listing.live { "live" } else { "down" },
        path = listing_path(listing),
        chip = status_chip(listing),
        price = price_tag(listing),
        spec = spec_html(listing, false),
        tags = tag_chips(listing),
        standby = standby_text(listing),
    )
}

/// `listings` as a grid of cards.
pub(crate) fn listing_grid(
    apps: &Apps,
    names: &HashMap<PublicKey, String>,
    listings: &[Listing],
    with_provider: bool,
) -> String {
    let cards: String = listings
        .iter()
        .map(|listing| listing_card(apps, names, listing, with_provider))
        .collect();
    format!("<div class=\"listing-grid\">{cards}</div>")
}

/// An empty section's note.
pub(crate) fn blankslate(text: &str) -> String {
    format!("<div class=\"blankslate\">{text}</div>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_are_grouped_by_thousands_and_nothing_is_rounded() {
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1_000), "1,000");
        assert_eq!(group(514_285), "514,285");
        assert_eq!(group(1_234_567), "1,234,567");
        let hour = |whole, remainder, interval| PerHour {
            whole,
            remainder,
            interval,
        };
        assert_eq!(hourly(hour(3_000, 0, 1)), "3,000 µUSDC");
        assert_eq!(hourly(hour(514_285, 5, 7)), "514,285 + 5/7 µUSDC");
    }

    #[test]
    fn a_lease_interval_reads_in_its_largest_whole_unit() {
        assert_eq!(interval(7), "7 s");
        assert_eq!(interval(60), "1 min");
        assert_eq!(interval(90), "90 s");
        assert_eq!(interval(600), "10 min");
        assert_eq!(interval(3_600), "1 h");
        assert_eq!(interval(5_400), "90 min");
        assert_eq!(interval(86_400), "1 day");
        assert_eq!(interval(172_800), "2 days");
        assert_eq!(interval(0), "0 s");
    }

    #[test]
    fn resources_read_in_a_readers_units_in_a_fixed_order() {
        let r = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            spec_items(
                &pairs
                    .iter()
                    .map(|(w, a)| ((*w).to_owned(), (*a).to_owned()))
                    .collect::<Vec<_>>(),
            )
        };
        let pairs = |items: Vec<(String, String)>| -> Vec<String> {
            items.into_iter().map(|(w, a)| format!("{w}={a}")).collect()
        };
        assert_eq!(
            pairs(r(&[
                ("storage_gb", "5"),
                ("memory_mb", "512"),
                ("cpu_millicores", "500"),
            ])),
            ["CPU=0.5 vCPU", "Memory=512 MB", "Storage=5 GB"]
        );
        assert_eq!(
            pairs(r(&[("cpu_millicores", "1250"), ("memory_mb", "4096")])),
            ["CPU=1.25 vCPU", "Memory=4 GB"]
        );
        assert_eq!(
            pairs(r(&[("vcpu", "2"), ("memory", "4 GiB"), ("gpu", "a100")])),
            ["CPU=2 vCPU", "Memory=4 GiB", "GPU=a100"]
        );
        assert_eq!(
            pairs(r(&[("bandwidth_tb", "1"), ("memory_mb", "1000")])),
            ["Memory=1000 MB", "Bandwidth tb=1"]
        );
    }
}

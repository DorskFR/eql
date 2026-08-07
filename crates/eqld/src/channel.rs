use crate::{config::Config, export::ExportError};

pub const SEPARATOR: char = '@';

pub fn variant_name(channel: &str, screen_w: i32, screen_h: i32) -> String {
    format!("{channel}{SEPARATOR}{screen_w}x{screen_h}")
}

pub fn parse_variant(name: &str, channel: &str) -> Option<(i32, i32)> {
    let rest = name.strip_prefix(channel)?.strip_prefix(SEPARATOR)?;
    let (width, height) = rest.split_once('x')?;
    let (width, height) = (width.parse().ok()?, height.parse().ok()?);
    (width > 0 && height > 0).then_some((width, height))
}

/// Aspect first, then area. A variant authored for the same shape reflows
/// correctly because positions are percentages; one of a different shape does
/// not, however close its pixel count.
pub fn pick<'a>(names: &'a [String], channel: &str, screen: (i32, i32)) -> Option<&'a str> {
    let (want_w, want_h) = screen;
    if want_w <= 0 || want_h <= 0 {
        return None;
    }
    let want_aspect = f64::from(want_w) / f64::from(want_h);
    let want_area = i64::from(want_w) * i64::from(want_h);

    names
        .iter()
        .filter_map(|name| parse_variant(name, channel).map(|screen| (name, screen)))
        .min_by(|(_, a), (_, b)| {
            let score = |(w, h): &(i32, i32)| {
                let aspect = (f64::from(*w) / f64::from(*h) - want_aspect).abs();
                let area = (i64::from(*w) * i64::from(*h) - want_area).abs();
                (aspect, area)
            };
            let (a_aspect, a_area) = score(a);
            let (b_aspect, b_area) = score(b);
            a_aspect
                .partial_cmp(&b_aspect)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a_area.cmp(&b_area))
        })
        .map(|(name, _)| name.as_str())
}

pub async fn list(config: &Config) -> Result<Vec<String>, ExportError> {
    let base = config.api.url.trim_end_matches('/');
    let url = format!("{base}/api/v1/layouts");
    let response = reqwest::Client::new()
        .get(&url)
        .bearer_auth(&config.api.token)
        .send()
        .await
        .map_err(|source| ExportError::Request {
            url: url.clone(),
            source,
        })?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|source| ExportError::Request {
            url: url.clone(),
            source,
        })?;
    if !status.is_success() {
        return Err(ExportError::Status {
            url,
            status,
            body: text.chars().take(300).collect(),
        });
    }
    let rows: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap_or_default();
    Ok(rows
        .into_iter()
        .filter_map(|row| row.get("name")?.as_str().map(str::to_string))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn a_variant_round_trips_through_its_name() {
        let name = variant_name("dorskui", 1280, 720);
        assert_eq!(name, "dorskui@1280x720");
        assert_eq!(parse_variant(&name, "dorskui"), Some((1280, 720)));
    }

    #[test]
    fn another_channel_is_not_mistaken_for_this_one() {
        assert_eq!(parse_variant("otherui@1280x720", "dorskui"), None);
        assert_eq!(parse_variant("dorskui", "dorskui"), None);
        assert_eq!(parse_variant("dorskui@bogus", "dorskui"), None);
        assert_eq!(parse_variant("dorskui@0x720", "dorskui"), None);
    }

    #[test]
    fn an_exact_resolution_wins() {
        let have = names(&["dorskui@3840x2160", "dorskui@1280x720", "dorskui@1920x1080"]);
        assert_eq!(
            pick(&have, "dorskui", (1280, 720)),
            Some("dorskui@1280x720")
        );
    }

    #[test]
    fn an_unlisted_resolution_falls_back_to_the_same_shape() {
        let have = names(&["dorskui@3840x2160", "dorskui@1440x1050"]);
        assert_eq!(
            pick(&have, "dorskui", (1920, 1080)),
            Some("dorskui@3840x2160"),
            "16:9 should beat 4:3 even though 4:3 is closer in pixel count"
        );
    }

    #[test]
    fn among_one_shape_the_closest_size_wins() {
        let have = names(&["dorskui@3840x2160", "dorskui@1280x720"]);
        assert_eq!(
            pick(&have, "dorskui", (1600, 900)),
            Some("dorskui@1280x720")
        );
    }

    #[test]
    fn layouts_outside_the_channel_are_ignored() {
        let have = names(&["default", "otherui@1280x720", "dorsk-erudin-1280x720-x"]);
        assert_eq!(pick(&have, "dorskui", (1280, 720)), None);
    }

    #[test]
    fn a_nonsense_screen_picks_nothing() {
        let have = names(&["dorskui@1280x720"]);
        assert_eq!(pick(&have, "dorskui", (0, 720)), None);
    }
}

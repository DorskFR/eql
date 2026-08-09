use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }

    pub fn within(&self, width: i32, height: i32) -> bool {
        self.x >= 0 && self.y >= 0 && self.x + self.w <= width && self.y + self.h <= height
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Layout(pub BTreeMap<String, (i32, i32, i32, i32)>);

impl Layout {
    pub fn rects(&self) -> impl Iterator<Item = (&str, Rect)> {
        self.0
            .iter()
            .map(|(name, &(x, y, w, h))| (name.as_str(), Rect { x, y, w, h }))
    }

    pub fn validate(&self, width: i32, height: i32, hidden: &[String]) -> Vec<String> {
        let rects: Vec<(&str, Rect)> = self
            .rects()
            .filter(|(name, _)| !hidden.iter().any(|window| window == name))
            .collect();
        let mut problems = Vec::new();
        for (name, rect) in &rects {
            if !rect.within(width, height) {
                problems.push(format!("{name} offscreen at {rect:?}"));
            }
        }
        for (i, (a_name, a)) in rects.iter().enumerate() {
            for (b_name, b) in &rects[i + 1..] {
                if a.overlaps(b) {
                    problems.push(format!("{a_name} overlaps {b_name}"));
                }
            }
        }
        problems
    }
}

/// Inverse of the skin builder's ini pass. `XRef`/`YRef` are deliberately
/// ignored: the generator preserves them rather than acting on them, so the
/// round-trip test — not an assumption about the client — is what makes this
/// safe. `sizes` is both the window set to look for and the fallback for
/// windows whose size lives in the skin XML rather than the ini.
pub fn from_ui_ini(
    text: &str,
    screen_w: i32,
    screen_h: i32,
    sizes: &BTreeMap<String, (i32, i32)>,
) -> Layout {
    let mut layout = BTreeMap::new();
    if screen_w <= 0 || screen_h <= 0 {
        return Layout(layout);
    }
    for (window, &(default_w, default_h)) in sizes {
        let Some(section) = section(text, window) else {
            continue;
        };
        let (Some(x_pct), Some(y_pct)) = (percent(section, "XPos"), percent(section, "YPos"))
        else {
            continue;
        };
        let w = integer(section, "Width").unwrap_or(default_w);
        let h = integer(section, "Height").unwrap_or(default_h);
        let x = (x_pct * f64::from(screen_w) / 100.0).round() as i32;
        let y = (y_pct * f64::from(screen_h) / 100.0).round() as i32;
        layout.insert(window.clone(), (x, y, w, h));
    }
    Layout(layout)
}

/// `split_inclusive` keeps the line terminator, so offsets stay exact against
/// the client's CRLF. Section names contain spaces (`[Chat 1]`).
fn section<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let header = format!("[{name}]");
    let mut offset = 0usize;
    let mut start = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        match start {
            None if trimmed == header => start = Some(offset + line.len()),
            Some(start) if trimmed.starts_with('[') => return Some(&text[start..offset]),
            _ => {}
        }
        offset += line.len();
    }
    start.map(|start| &text[start..])
}

fn value<'a>(section: &'a str, key: &str) -> Option<&'a str> {
    section
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
}

fn percent(section: &str, key: &str) -> Option<f64> {
    value(section, key)?.trim_end_matches('%').parse().ok()
}

fn integer(section: &str, key: &str) -> Option<i32> {
    value(section, key)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes(pairs: &[(&str, (i32, i32))]) -> BTreeMap<String, (i32, i32)> {
        pairs
            .iter()
            .map(|(name, wh)| ((*name).to_string(), *wh))
            .collect()
    }

    const INI: &str = "[PlayerWindow]\r\nXRef=left\r\nXPos=25.000000%\r\nYPos=50.000000%\r\nWidth=300\r\nHeight=130\r\n[Chat 1]\r\nXRef=center\r\nXPos=0.000000%\r\nYPos=0.000000%\r\nWidth=840\r\nHeight=140\r\n[BuffWindow]\r\nXPos=50.000000%\r\nYPos=0.000000%\r\n";

    #[test]
    fn percentages_become_pixels_against_the_named_screen() {
        let got = from_ui_ini(INI, 1280, 720, &sizes(&[("PlayerWindow", (0, 0))]));
        assert_eq!(got.0["PlayerWindow"], (320, 360, 300, 130));
    }

    #[test]
    fn a_section_name_may_contain_a_space() {
        let got = from_ui_ini(INI, 1280, 720, &sizes(&[("Chat 1", (0, 0))]));
        assert_eq!(got.0["Chat 1"], (0, 0, 840, 140));
    }

    #[test]
    fn a_window_the_ini_does_not_size_falls_back_to_the_skin_xml() {
        let got = from_ui_ini(INI, 1280, 720, &sizes(&[("BuffWindow", (780, 150))]));
        assert_eq!(got.0["BuffWindow"], (640, 0, 780, 150));
    }

    #[test]
    fn a_window_the_ini_does_not_position_is_left_out() {
        let got = from_ui_ini(INI, 1280, 720, &sizes(&[("PetInfoWindow", (300, 110))]));
        assert!(got.0.is_empty());
    }

    #[test]
    fn only_the_asked_for_windows_come_back() {
        let got = from_ui_ini(INI, 1280, 720, &sizes(&[("PlayerWindow", (0, 0))]));
        assert_eq!(got.0.len(), 1, "{got:?}");
    }

    #[test]
    fn a_nonsense_screen_yields_nothing_rather_than_dividing_by_it() {
        assert!(
            from_ui_ini(INI, 0, 720, &sizes(&[("PlayerWindow", (0, 0))]))
                .0
                .is_empty()
        );
    }
}

/// Content scale, which the window rects cannot express: the template bakes
/// 225 `<Font>` tags and a 64px spell gem, and a phone and a 4K monitor want
/// different ones at the same pixel count. Empty means leave the template be.
/// `hidden` and `bare` name ini sections, not layout windows, so they also
/// reach panels the skin never positions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub font_shift: i32,
    pub gem: Option<i32>,
    pub hidden: Vec<String>,
    pub bare: Vec<String>,
}

impl Style {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn hides(&self, section: &str) -> bool {
        self.hidden.iter().any(|name| name == section)
    }

    pub fn strips(&self, section: &str) -> bool {
        self.bare.iter().any(|name| name == section)
    }

    /// EQ only ships fonts 1..=5; a shift that would leave that range clamps
    /// rather than producing a tag the client silently ignores.
    pub fn shift_font(&self, font: i32) -> i32 {
        (font + self.font_shift).clamp(1, 5)
    }

    /// The template draws a 40px icon inside a 64px gem; a resized gem keeps
    /// that ratio so the art stays centred.
    pub fn icon_for(&self, gem: i32) -> i32 {
        (i64::from(gem) * 40 / 64) as i32
    }
}

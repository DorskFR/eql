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

    pub fn validate(&self, width: i32, height: i32) -> Vec<String> {
        let rects: Vec<(&str, Rect)> = self.rects().collect();
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

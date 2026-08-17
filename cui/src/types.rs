use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BBox {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl BBox {
    #[allow(dead_code)]
    pub const ZERO: Self = Self {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    };

    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }


    pub fn union(&self, other: &Self) -> Self {
        let x_min = self.x.min(other.x);
        let y_min = self.y.min(other.y);
        let x_max = (self.x + self.width).max(other.x + other.width);
        let y_max = (self.y + self.height).max(other.y + other.height);
        Self {
            x: x_min,
            y: y_min,
            width: x_max - x_min,
            height: y_max - y_min,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchState {
    SearchStart,
    SearchEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchResult {
    Win,
    Lose,
    Unknown,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub result: MatchResult,
    pub p1_name: Option<String>,
    pub p2_name: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcMessage {
    Log {
        message: String,
    },
    Progress {
        phase: String,
        current: usize,
        total: usize,
        percentage: f32,
        message: String,
    },
    SegmentDetected {
        index: usize,
        start: f64,
        end: f64,
        result: MatchResult,
        p1: Option<String>,
        p2: Option<String>,
    },
    Done {
        total_frames: usize,
        segments_count: usize,
        calc_time_secs: u64,
        export_time_secs: u64,
        total_time_secs: u64,
        output_dir: String,
    },
    Error {
        message: String,
    },
}

// SPDX-License-Identifier: GPL-3.0-only

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::chat) enum TimelineVerbosity {
    Timeline,
    Replay,
    Debug,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::chat) struct TimelineRequest {
    pub(in crate::chat) turn_filter: Option<String>,
    pub(in crate::chat) kind_filter: Option<String>,
    pub(in crate::chat) point_filter: Option<String>,
    pub(in crate::chat) page: usize,
}

impl TimelineRequest {
    pub(in crate::chat) fn for_turn(turn_id: &str) -> Self {
        Self {
            turn_filter: Some(turn_id.to_owned()),
            kind_filter: None,
            point_filter: None,
            page: 1,
        }
    }
}

impl Default for TimelineRequest {
    fn default() -> Self {
        Self {
            turn_filter: None,
            kind_filter: None,
            point_filter: None,
            page: 1,
        }
    }
}

// SPDX-License-Identifier: GPL-3.0-only

mod cells;
mod classification;
mod events;
mod filter;
mod human;
mod output;
mod request;
mod trace;

pub(super) use cells::{screen_coding_cells, screen_failure_cells, screen_timeline_cells};
pub(in crate::chat) use output::{print_replay_status, print_replay_status_for_human};
pub(in crate::chat) use request::{TimelineRequest, TimelineVerbosity};
pub(super) use trace::print_screen_trace_snapshot;
pub(in crate::chat) use trace::{print_trace_status, print_trace_status_for_human};

#[cfg(test)]
pub(super) use cells::{
    screen_coding_cells_from_replay, screen_failure_cells_from_replay,
    screen_timeline_cells_from_replay,
};
#[cfg(test)]
pub(super) use filter::timeline_point_matches;

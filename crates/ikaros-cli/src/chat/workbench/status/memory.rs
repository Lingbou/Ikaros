// SPDX-License-Identifier: GPL-3.0-only

mod cells;
mod output;
mod projection;
mod value;

pub(super) use cells::screen_memory_cell;
pub(in crate::chat) use output::{
    memory_status_human_lines, print_memory_status, print_memory_status_for_human,
};

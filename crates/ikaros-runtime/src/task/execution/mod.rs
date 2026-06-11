// SPDX-License-Identifier: GPL-3.0-only

mod agent_loop;
mod automation;
mod deterministic;
mod reporting;

pub use automation::execute_task_for_automation;
pub use deterministic::{execute_task_text, execute_task_text_with_options};

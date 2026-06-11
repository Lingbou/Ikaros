// SPDX-License-Identifier: GPL-3.0-only

mod execution;
mod planning;
mod report;
mod types;

pub use execution::{
    execute_task_for_automation, execute_task_text, execute_task_text_with_options,
};
pub use planning::{build_task_plan, task_steps};
pub use report::{task_report_succeeded, task_report_summary};
pub use types::{RuntimeTaskExecution, RuntimeTaskPlan, TaskRunOptions};

#[cfg(test)]
mod tests;

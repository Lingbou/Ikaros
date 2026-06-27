// SPDX-License-Identifier: GPL-3.0-only

use crate::terminal_inline;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchLineInputUi {
    pub(super) model_label: String,
    pub(super) workspace_label: String,
    pub(super) show_intro: bool,
}

impl WorkbenchLineInputUi {
    pub fn new(
        model_label: impl Into<String>,
        workspace_label: impl Into<String>,
        show_intro: bool,
    ) -> Self {
        Self {
            model_label: terminal_inline(&model_label.into()),
            workspace_label: terminal_inline(&workspace_label.into()),
            show_intro,
        }
    }
}

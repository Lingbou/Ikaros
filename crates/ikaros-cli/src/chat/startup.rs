// SPDX-License-Identifier: GPL-3.0-only

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::chat) enum WorkbenchStartupScreen {
    None,
    Inline,
    Fullscreen,
}

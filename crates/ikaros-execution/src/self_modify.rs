// SPDX-License-Identifier: GPL-3.0-only

mod apply;
mod checks;
mod diff;
mod dry_run;
mod rollback;
mod store;
mod types;

pub use store::SelfModifyStore;
pub use types::{
    SelfModifyApplyReport, SelfModifyChangeKind, SelfModifyCheckProfile, SelfModifyCheckReport,
    SelfModifyDryRunReport, SelfModifyHeartbeatReport, SelfModifyOperationKind,
    SelfModifyOperationRecord, SelfModifyProposal, SelfModifyRollbackPlan,
    SelfModifyRollbackReport,
};

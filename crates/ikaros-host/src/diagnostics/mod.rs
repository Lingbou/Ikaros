// SPDX-License-Identifier: GPL-3.0-only

mod doctor;
mod init;
mod provider;
mod sandbox;
mod types;

pub use doctor::runtime_doctor_report;
pub use init::{initialize_runtime_home, initialize_runtime_home_with_options};
pub use provider::{
    ProviderDebugMatrixReport, ProviderHealthReport, ProviderInspectFallbackRow,
    ProviderInspectReport, ProviderMatrixReport, ProviderMatrixRow, ProviderProfileCatalogRow,
    ProviderProfilesReport, WorkbenchModelBudgetStatus, WorkbenchModelCostStatus,
    WorkbenchProviderFallbackStatus, WorkbenchProviderHealthStatus, WorkbenchProviderStatusReport,
    model_budget_status_report, provider_debug_matrix_report, provider_debug_report,
    provider_health_report, provider_inspect_report, provider_matrix_report,
    provider_profiles_report, workbench_provider_status_report,
};
pub use sandbox::{configured_sandbox_debug_report, debug_sandbox_report, sandbox_probe};
pub use types::{
    AgentSummary, ExecutionSummary, ModelSummary, PersonaSummary, PluginSummary, RagSummary,
    RuntimeDoctorReport, RuntimeInitReport, StoreSummary,
};

#[cfg(test)]
mod tests;

// SPDX-License-Identifier: GPL-3.0-only

use super::{IkarosConfig, ModelProviderKind, ModelTransportKind};

impl IkarosConfig {
    /// Expands `preset:` fields into `provider`/`transport`/`compat_profile`.
    ///
    /// Run after shape loading but before validation so that presets fill in the
    /// provider triple while still allowing explicit field values to win. Only
    /// known presets are expanded; an unknown preset is left untouched for the
    /// validator to report.
    pub fn expand_presets(&mut self) {
        if let Some(preset_id) = self.model.default.preset.as_deref() {
            expand_single_preset(
                &mut self.model.default.provider,
                &mut self.model.default.transport,
                &mut self.model.default.compat_profile,
                preset_id,
            );
        }
        for fallback in &mut self.model.default.fallbacks {
            if let Some(preset_id) = fallback.preset.as_deref() {
                expand_single_preset(
                    &mut fallback.provider,
                    &mut fallback.transport,
                    &mut fallback.compat_profile,
                    preset_id,
                );
            }
        }
    }
}

fn expand_single_preset(
    provider: &mut ModelProviderKind,
    transport: &mut ModelTransportKind,
    compat_profile: &mut String,
    preset_id: &str,
) {
    let Ok(spec) = crate::preset::resolve_preset(preset_id) else {
        return;
    };
    if matches!(provider, ModelProviderKind::OpenaiCompatible) {
        if let Ok(kind) = ModelProviderKind::parse(spec.provider) {
            *provider = kind;
        }
    }
    if provider.as_str() != spec.provider {
        return;
    }
    if matches!(
        transport,
        ModelTransportKind::OpenaiCompatibleChatCompletions
    ) {
        if let Ok(kind) = ModelTransportKind::parse(spec.transport) {
            *transport = kind;
        }
    }
    if compat_profile.is_empty() || compat_profile == "auto" {
        *compat_profile = spec.compat_profile.into();
    }
}

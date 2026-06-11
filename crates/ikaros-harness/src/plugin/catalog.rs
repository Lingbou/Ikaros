// SPDX-License-Identifier: GPL-3.0-only

use super::{
    issue::PluginLoadIssue,
    loader::{
        discover_plugin_manifests, load_plugin_manifest, load_plugin_marketplace, marketplace_path,
    },
    manifest::{PluginManifest, PluginSkillManifest},
    marketplace::PluginMarketplaceEntry,
};
use ikaros_core::{Result, redact_secrets};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoadedPluginManifest {
    pub path: PathBuf,
    pub manifest: PluginManifest,
    pub marketplace: PluginMarketplaceEntry,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PluginCatalog {
    pub plugins: Vec<LoadedPluginManifest>,
    pub warnings: Vec<PluginLoadIssue>,
}

impl PluginCatalog {
    pub fn load(skills_dir: impl AsRef<Path>) -> Result<Self> {
        let skills_dir = skills_dir.as_ref();
        if !skills_dir.exists() {
            return Ok(Self::default());
        }
        let mut catalog = Self::default();
        let marketplace = match load_plugin_marketplace(skills_dir) {
            Ok(marketplace) => marketplace,
            Err(issue) => {
                catalog.warnings.push(issue);
                Default::default()
            }
        };
        let mut matched_marketplace_entries = BTreeSet::new();
        for manifest_path in discover_plugin_manifests(skills_dir)? {
            match load_plugin_manifest(&manifest_path) {
                Ok(manifest) => {
                    let marketplace_entry = marketplace
                        .get(&manifest.name)
                        .cloned()
                        .unwrap_or_else(|| PluginMarketplaceEntry::local_default(&manifest.name));
                    matched_marketplace_entries.insert(manifest.name.clone());
                    catalog.plugins.push(LoadedPluginManifest {
                        path: manifest_path,
                        manifest,
                        marketplace: marketplace_entry,
                    });
                }
                Err(error) => catalog.warnings.push(PluginLoadIssue {
                    path: manifest_path,
                    message: redact_secrets(&error.to_string()),
                }),
            }
        }
        for name in marketplace.keys() {
            if !matched_marketplace_entries.contains(name) {
                catalog.warnings.push(PluginLoadIssue {
                    path: marketplace_path(skills_dir),
                    message: format!("marketplace entry has no matching plugin manifest: {name}"),
                });
            }
        }
        catalog.plugins.sort_by(|left, right| {
            left.marketplace
                .priority
                .cmp(&right.marketplace.priority)
                .then_with(|| left.manifest.name.cmp(&right.manifest.name))
        });
        Ok(catalog)
    }

    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }

    pub fn declared_skill_count(&self) -> usize {
        self.plugins
            .iter()
            .filter(|plugin| plugin.marketplace.enabled)
            .map(|plugin| plugin.manifest.skills.len())
            .sum()
    }

    pub fn enabled_plugin_count(&self) -> usize {
        self.plugins
            .iter()
            .filter(|plugin| plugin.marketplace.enabled)
            .count()
    }

    pub fn disabled_plugin_count(&self) -> usize {
        self.plugins
            .iter()
            .filter(|plugin| !plugin.marketplace.enabled)
            .count()
    }

    pub fn declared_skill_names(&self) -> Vec<String> {
        let mut names = self
            .plugins
            .iter()
            .filter(|plugin| plugin.marketplace.enabled)
            .flat_map(|plugin| {
                plugin
                    .manifest
                    .skills
                    .iter()
                    .map(move |skill| format!("{}.{}", plugin.manifest.name, skill.name))
            })
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    pub fn find_skill(&self, name: &str) -> Option<(&LoadedPluginManifest, &PluginSkillManifest)> {
        self.find_skill_by_enabled_state(name, true)
    }

    pub fn find_declared_skill(
        &self,
        name: &str,
    ) -> Option<(&LoadedPluginManifest, &PluginSkillManifest)> {
        self.find_skill_by_enabled_state(name, false)
    }

    fn find_skill_by_enabled_state(
        &self,
        name: &str,
        enabled_only: bool,
    ) -> Option<(&LoadedPluginManifest, &PluginSkillManifest)> {
        self.plugins.iter().find_map(|plugin| {
            if enabled_only && !plugin.marketplace.enabled {
                return None;
            }
            plugin.manifest.skills.iter().find_map(|skill| {
                let qualified = format!("{}.{}", plugin.manifest.name, skill.name);
                if skill.name == name || qualified == name {
                    Some((plugin, skill))
                } else {
                    None
                }
            })
        })
    }
}

// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosConfig, RemoteProviderConfig};
use url::Url;

pub fn provider_egress_allowed_hosts(config: &IkarosConfig) -> Vec<String> {
    let mut hosts = Vec::new();
    if config.execution.network.allow_provider_hosts {
        push_provider_host(&mut hosts, &config.effective_model_provider());
        for instance in config.agent.instances.values() {
            let model = instance.model.as_ref().unwrap_or(&config.model.default);
            let provider = instance
                .providers
                .model
                .as_ref()
                .unwrap_or(&config.providers.model);
            push_provider_host(&mut hosts, &model.effective_provider_config(provider));
        }
        push_provider_host(&mut hosts, &config.providers.embedding);
        push_provider_host(&mut hosts, &config.providers.tts);
        push_provider_host(&mut hosts, &config.providers.asr);
        push_provider_host(&mut hosts, &config.providers.search);
        hosts.extend([
            "duckduckgo.com".into(),
            "api.search.brave.com".into(),
            "api.bing.microsoft.com".into(),
            "serpapi.com".into(),
            "api.tavily.com".into(),
        ]);
        if config.model.default.provider.eq_ignore_ascii_case("ollama")
            && config.effective_model_provider().base_url.trim().is_empty()
        {
            hosts.push("127.0.0.1".into());
            hosts.push("localhost".into());
        }
    }
    hosts.extend(config.execution.network.allowed_hosts.iter().cloned());
    hosts.sort();
    hosts.dedup();
    hosts
}

fn push_provider_host(hosts: &mut Vec<String>, provider: &RemoteProviderConfig) {
    let base_url = provider.base_url.trim();
    if base_url.is_empty() {
        return;
    }
    if let Ok(parsed) = Url::parse(base_url)
        && let Some(host) = parsed.host_str()
    {
        hosts.push(host.trim_end_matches('.').to_ascii_lowercase());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikaros_core::ModelConfig;

    #[test]
    fn provider_egress_hosts_include_configured_providers_without_paths() {
        let mut config = IkarosConfig::default();
        config.providers.model.base_url = "https://api.example/v1".into();
        config.providers.embedding.base_url = "https://embedding.example/embeddings".into();
        config.execution.network.allowed_hosts = vec!["extra.example".into()];

        let hosts = provider_egress_allowed_hosts(&config);

        assert!(hosts.contains(&"api.example".into()));
        assert!(hosts.contains(&"embedding.example".into()));
        assert!(hosts.contains(&"extra.example".into()));
        assert!(!hosts.contains(&"https://api.example/v1".into()));
    }

    #[test]
    fn provider_egress_hosts_allow_default_local_ollama() {
        let mut config = IkarosConfig::default();
        config.model.default = ModelConfig {
            provider: "ollama".into(),
            model: "llama3.2".into(),
            transport: "ollama-chat".into(),
            ..ModelConfig::default()
        };

        let hosts = provider_egress_allowed_hosts(&config);

        assert!(hosts.contains(&"127.0.0.1".into()));
        assert!(hosts.contains(&"localhost".into()));
    }
}

// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};
use std::{fmt, ops::Deref};

macro_rules! string_enum {
    ($name:ident, $default:ident, { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
        #[serde(rename_all = "kebab-case")]
        pub enum $name {
            $($variant,)+
        }

        impl Default for $name {
            fn default() -> Self {
                Self::$default
            }
        }

        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self {
                    $(Self::$variant => $value,)+
                }
            }

            pub fn parse(value: &str) -> std::result::Result<Self, String> {
                match value.trim() {
                    $($value => Ok(Self::$variant),)+
                    other => Err(format!("unsupported {} `{other}`", stringify!($name))),
                }
            }
        }

        impl Deref for $name {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::parse(value).unwrap_or_else(|message| panic!("{message}"))
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::from(value.as_str())
            }
        }
    };
}

string_enum!(ModelProviderKind, OpenaiCompatible, {
    OpenaiCompatible => "openai-compatible",
    Anthropic => "anthropic",
    Ollama => "ollama",
    Mock => "mock",
});

string_enum!(ModelTransportKind, OpenaiCompatibleChatCompletions, {
    OpenaiCompatibleChatCompletions => "openai-compatible-chat-completions",
    AnthropicMessages => "anthropic-messages",
    OllamaChat => "ollama-chat",
    Mock => "mock",
});

string_enum!(StoreBackend, Jsonl, {
    Jsonl => "jsonl",
    Sqlite => "sqlite",
});

string_enum!(EmbeddingProviderKind, Hash, {
    OpenaiCompatible => "openai-compatible",
    Ollama => "ollama",
    Hash => "hash",
    Sparse => "sparse",
    Mock => "mock",
});

string_enum!(VoiceProviderKind, Mock, {
    OpenaiCompatible => "openai-compatible",
    Mock => "mock",
});

string_enum!(SandboxBackend, Local, {
    Local => "local",
    DryRun => "dry-run",
    Docker => "docker",
});

string_enum!(SandboxReadScope, Workspace, {
    Workspace => "workspace",
    Host => "host",
});

use super::{Integration, IntegrationState, IntegrationStatus, ProviderFailureCategory};
use std::collections::BTreeMap;

#[derive(Clone)]
struct RegistryEntry {
    state: IntegrationState,
    last_success_at: Option<String>,
    safe_message: String,
}

pub(super) struct ProviderRegistry {
    entries: BTreeMap<Integration, RegistryEntry>,
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        let entries = [
            (
                Integration::Openai,
                RegistryEntry {
                    state: IntegrationState::Unavailable,
                    last_success_at: None,
                    safe_message: "尚未配置 OpenAI 凭据。".to_owned(),
                },
            ),
            (
                Integration::AppleMusic,
                RegistryEntry {
                    state: IntegrationState::Unavailable,
                    last_success_at: None,
                    safe_message: "尚未连接系统媒体会话。".to_owned(),
                },
            ),
            (
                Integration::Musicbrainz,
                RegistryEntry {
                    state: IntegrationState::Disabled,
                    last_success_at: None,
                    safe_message: "音乐元数据服务已关闭。".to_owned(),
                },
            ),
            (
                Integration::Weather,
                RegistryEntry {
                    state: IntegrationState::Disabled,
                    last_success_at: None,
                    safe_message: "天气服务已关闭。".to_owned(),
                },
            ),
        ]
        .into_iter()
        .collect();
        Self { entries }
    }
}

impl ProviderRegistry {
    pub(super) fn credential_present(&mut self) {
        if let Some(entry) = self.entries.get_mut(&Integration::Openai)
            && entry.state == IntegrationState::Unavailable
        {
            entry.state = IntegrationState::Connected;
            "凭据已保存；最近验证时间不可用。".clone_into(&mut entry.safe_message);
        }
    }

    pub(super) fn set_enabled(&mut self, integration: Integration, enabled: bool) {
        if let Some(entry) = self.entries.get_mut(&integration) {
            if enabled && entry.state == IntegrationState::Disabled {
                entry.state = IntegrationState::Unavailable;
                "尚未测试连接。".clone_into(&mut entry.safe_message);
            } else if !enabled {
                entry.state = IntegrationState::Disabled;
                "此集成已关闭。".clone_into(&mut entry.safe_message);
            }
        }
    }

    pub(super) fn configured(&mut self, verified_at: String) {
        if let Some(entry) = self.entries.get_mut(&Integration::Openai) {
            entry.state = IntegrationState::Connected;
            entry.last_success_at = Some(verified_at);
            "OpenAI 连接可用。".clone_into(&mut entry.safe_message);
        }
    }

    pub(super) fn secret_deleted(&mut self) {
        if let Some(entry) = self.entries.get_mut(&Integration::Openai) {
            entry.state = IntegrationState::Unavailable;
            entry.last_success_at = None;
            "尚未配置 OpenAI 凭据。".clone_into(&mut entry.safe_message);
        }
    }

    pub(super) fn provider_success(&mut self, integration: Integration, at: String) {
        if let Some(entry) = self.entries.get_mut(&integration) {
            entry.state = IntegrationState::Connected;
            entry.last_success_at = Some(at);
            "连接可用。".clone_into(&mut entry.safe_message);
        }
    }

    pub(super) fn provider_failure(
        &mut self,
        integration: Integration,
        category: ProviderFailureCategory,
    ) {
        if let Some(entry) = self.entries.get_mut(&integration) {
            entry.state = match category {
                ProviderFailureCategory::Authentication
                | ProviderFailureCategory::InvalidResponse
                | ProviderFailureCategory::RateLimit
                | ProviderFailureCategory::Timeout
                | ProviderFailureCategory::Unavailable => IntegrationState::Degraded,
            };
            match category {
                ProviderFailureCategory::Authentication => "服务认证失败。",
                ProviderFailureCategory::RateLimit => "服务请求过于频繁。",
                ProviderFailureCategory::Timeout => "服务响应超时。",
                ProviderFailureCategory::Unavailable => "服务或网络当前不可用。",
                ProviderFailureCategory::InvalidResponse => "服务返回了无效内容。",
            }
            .clone_into(&mut entry.safe_message);
        }
    }

    pub(super) fn statuses(&self) -> Vec<IntegrationStatus> {
        [
            Integration::Openai,
            Integration::AppleMusic,
            Integration::Musicbrainz,
            Integration::Weather,
        ]
        .into_iter()
        .filter_map(|integration| {
            self.entries
                .get(&integration)
                .map(|entry| IntegrationStatus {
                    integration,
                    state: entry.state,
                    last_success_at: entry.last_success_at.clone(),
                    safe_message: entry.safe_message.clone(),
                })
        })
        .collect()
    }
}

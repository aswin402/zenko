use std::collections::BTreeMap;
use zenko_core::{
    GATEWAY_REGISTRY, PROVIDER_REGISTRY, STREAMING_REGISTRY, StreamingOptionId, ZenkoConfig,
    find_gateway, find_provider,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderField {
    BaseUrl,
    ApiKey,
    LeadModel,
}

pub const PROVIDER_FLOW_STEPS: &[ProviderField] = &[
    ProviderField::BaseUrl,
    ProviderField::ApiKey,
    ProviderField::LeadModel,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    Root { cursor: usize },
    Providers { cursor: usize },
    ProviderInput {
        provider_id: &'static str,
        step_idx: usize,
        buffer: String,
    },
    Streaming { cursor: usize },
    Gateways { cursor: usize },
    GatewayMenu {
        gateway_id: &'static str,
        cursor: usize,
    },
    GatewayTokenInput {
        gateway_id: &'static str,
        buffer: String,
    },
}

#[derive(Debug, Clone)]
pub struct MenuEntry {
    pub label: String,
    pub detail: String,
    pub badge: String,
    pub badge_active: bool,
    pub action: MenuAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    OpenProviders,
    OpenStreaming,
    OpenGateways,
    SaveAndFinish,
    SelectProvider(&'static str),
    ToggleStreaming(StreamingOptionId),
    OpenGateway(&'static str),
    ToggleGatewayEnabled(&'static str),
    EditGatewayToken(&'static str),
    GoBack,
}

#[derive(Debug, Clone)]
pub struct InputPromptView {
    pub title: String,
    pub subtitle: String,
    pub value: String,
    pub masked: bool,
    pub step_label: String,
    pub placeholder: String,
}

#[derive(Debug, Clone)]
pub enum ScreenView {
    Menu {
        title: String,
        subtitle: String,
        cursor: usize,
        items: Vec<MenuEntry>,
    },
    Input(InputPromptView),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineOutcome {
    Continue,
    SaveAndExit,
    CancelAndExit,
}

#[derive(Debug, Clone)]
pub struct SetupDraft {
    pub config: ZenkoConfig,
    pub existing_secrets: BTreeMap<String, bool>,
    pub pending_secrets: BTreeMap<String, String>,
    pub status_notice: Option<String>,
}

impl SetupDraft {
    pub fn has_secret(&self, vault_key: &str) -> bool {
        let pending_non_empty = self
            .pending_secrets
            .get(vault_key)
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        pending_non_empty || *self.existing_secrets.get(vault_key).unwrap_or(&false)
    }

    pub fn streaming_value(&self, id: StreamingOptionId) -> bool {
        match id {
            StreamingOptionId::ResponseStreaming => self.config.provider.enable_streaming,
            StreamingOptionId::PromptCaching => self.config.provider.enable_prompt_caching,
            StreamingOptionId::StickySessionRouting => self.config.provider.sticky_session_routing,
            StreamingOptionId::TelegramDraftStreaming => self.config.telegram.stream_drafts,
        }
    }

    pub fn toggle_streaming(&mut self, id: StreamingOptionId) {
        match id {
            StreamingOptionId::ResponseStreaming => {
                self.config.provider.enable_streaming = !self.config.provider.enable_streaming;
            }
            StreamingOptionId::PromptCaching => {
                self.config.provider.enable_prompt_caching = !self.config.provider.enable_prompt_caching;
            }
            StreamingOptionId::StickySessionRouting => {
                self.config.provider.sticky_session_routing = !self.config.provider.sticky_session_routing;
            }
            StreamingOptionId::TelegramDraftStreaming => {
                self.config.telegram.stream_drafts = !self.config.telegram.stream_drafts;
            }
        }
    }

    pub fn gateway_enabled(&self, gateway_id: &str) -> bool {
        match gateway_id {
            "telegram" => self.config.telegram.enabled,
            "webhooks" => self.config.webhooks.enabled,
            _ => false,
        }
    }

    pub fn toggle_gateway(&mut self, gateway_id: &str) {
        match gateway_id {
            "telegram" => self.config.telegram.enabled = !self.config.telegram.enabled,
            "webhooks" => self.config.webhooks.enabled = !self.config.webhooks.enabled,
            _ => {}
        }
    }
}

/// Stack-based setup navigation state machine (zero `if/else` spaghetti, clean Back support).
pub struct SetupMachine {
    stack: Vec<Screen>,
    pub draft: SetupDraft,
}

impl SetupMachine {
    pub fn new(config: ZenkoConfig, existing_secrets: BTreeMap<String, bool>) -> Self {
        Self {
            stack: vec![Screen::Root { cursor: 0 }],
            draft: SetupDraft {
                config,
                existing_secrets,
                pending_secrets: BTreeMap::new(),
                status_notice: None,
            },
        }
    }

    pub fn breadcrumbs(&self) -> Vec<String> {
        self.stack
            .iter()
            .map(|s| match s {
                Screen::Root { .. } => "Setup".to_string(),
                Screen::Providers { .. } => "Providers".to_string(),
                Screen::ProviderInput { provider_id, .. } => {
                    find_provider(provider_id).label.to_string()
                }
                Screen::Streaming { .. } => "Streaming".to_string(),
                Screen::Gateways { .. } => "Gateway".to_string(),
                Screen::GatewayMenu { gateway_id, .. }
                | Screen::GatewayTokenInput { gateway_id, .. } => find_gateway(gateway_id)
                    .map(|g| g.label.to_string())
                    .unwrap_or_else(|| (*gateway_id).to_string()),
            })
            .collect()
    }

    pub fn is_input_mode(&self) -> bool {
        matches!(
            self.stack.last(),
            Some(Screen::ProviderInput { .. } | Screen::GatewayTokenInput { .. })
        )
    }

    pub fn current_view(&self) -> ScreenView {
        match self.stack.last() {
            Some(Screen::Root { cursor }) => self.view_root(*cursor),
            Some(Screen::Providers { cursor }) => self.view_providers(*cursor),
            Some(Screen::ProviderInput {
                provider_id,
                step_idx,
                buffer,
            }) => self.view_provider_input(provider_id, *step_idx, buffer),
            Some(Screen::Streaming { cursor }) => self.view_streaming(*cursor),
            Some(Screen::Gateways { cursor }) => self.view_gateways(*cursor),
            Some(Screen::GatewayMenu { gateway_id, cursor }) => {
                self.view_gateway_menu(gateway_id, *cursor)
            }
            Some(Screen::GatewayTokenInput { gateway_id, buffer }) => {
                self.view_gateway_token_input(gateway_id, buffer)
            }
            None => self.view_root(0),
        }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let len = match self.current_view() {
            ScreenView::Menu { items, .. } => items.len(),
            ScreenView::Input(_) => return,
        };
        if len == 0 {
            return;
        }
        if let Some(
            Screen::Root { cursor }
            | Screen::Providers { cursor }
            | Screen::Streaming { cursor }
            | Screen::Gateways { cursor }
            | Screen::GatewayMenu { cursor, .. },
        ) = self.stack.last_mut()
        {
            let next = (*cursor as isize + delta).rem_euclid(len as isize);
            *cursor = next as usize;
        }
    }

    pub fn push_char(&mut self, ch: char) {
        match self.stack.last_mut() {
            Some(Screen::ProviderInput { buffer, .. })
            | Some(Screen::GatewayTokenInput { buffer, .. }) => buffer.push(ch),
            _ => {}
        }
    }

    pub fn paste_text(&mut self, text: &str) {
        let cleaned: String = text.chars().filter(|c| !matches!(c, '\r' | '\n')).collect();
        match self.stack.last_mut() {
            Some(Screen::ProviderInput { buffer, .. })
            | Some(Screen::GatewayTokenInput { buffer, .. }) => buffer.push_str(&cleaned),
            _ => {}
        }
    }

    pub fn backspace(&mut self) {
        match self.stack.last_mut() {
            Some(Screen::ProviderInput { buffer, .. })
            | Some(Screen::GatewayTokenInput { buffer, .. }) => {
                buffer.pop();
            }
            _ => {}
        }
    }

    pub fn go_back(&mut self) -> MachineOutcome {
        let Some(top) = self.stack.pop() else {
            return MachineOutcome::CancelAndExit;
        };
        match top {
            Screen::ProviderInput {
                provider_id,
                step_idx,
                ..
            } if step_idx > 0 => {
                let prev = step_idx - 1;
                let buffer =
                    self.initial_provider_field_value(provider_id, PROVIDER_FLOW_STEPS[prev]);
                self.stack.push(Screen::ProviderInput {
                    provider_id,
                    step_idx: prev,
                    buffer,
                });
                MachineOutcome::Continue
            }
            _ => match self.stack.is_empty() {
                true => {
                    self.stack.push(top);
                    MachineOutcome::CancelAndExit
                }
                false => MachineOutcome::Continue,
            },
        }
    }

    pub fn confirm(&mut self) -> MachineOutcome {
        match self.current_view() {
            ScreenView::Menu { cursor, items, .. } => {
                let Some(item) = items.get(cursor) else {
                    return MachineOutcome::Continue;
                };
                self.apply_action(item.action.clone())
            }
            ScreenView::Input(_) => self.confirm_input(),
        }
    }

    fn apply_action(&mut self, action: MenuAction) -> MachineOutcome {
        self.draft.status_notice = None;
        match action {
            MenuAction::OpenProviders => {
                let initial = PROVIDER_REGISTRY
                    .iter()
                    .position(|p| p.id == self.draft.config.provider.kind)
                    .unwrap_or(0);
                self.stack.push(Screen::Providers { cursor: initial });
                MachineOutcome::Continue
            }
            MenuAction::OpenStreaming => {
                self.stack.push(Screen::Streaming { cursor: 0 });
                MachineOutcome::Continue
            }
            MenuAction::OpenGateways => {
                self.stack.push(Screen::Gateways { cursor: 0 });
                MachineOutcome::Continue
            }
            MenuAction::SaveAndFinish => MachineOutcome::SaveAndExit,
            MenuAction::SelectProvider(provider_id) => {
                let initial_buf = self.initial_provider_field_value(provider_id, PROVIDER_FLOW_STEPS[0]);
                self.stack.push(Screen::ProviderInput {
                    provider_id,
                    step_idx: 0,
                    buffer: initial_buf,
                });
                MachineOutcome::Continue
            }
            MenuAction::ToggleStreaming(opt) => {
                self.draft.toggle_streaming(opt);
                MachineOutcome::Continue
            }
            MenuAction::OpenGateway(gateway_id) => {
                self.stack.push(Screen::GatewayMenu {
                    gateway_id,
                    cursor: 0,
                });
                MachineOutcome::Continue
            }
            MenuAction::ToggleGatewayEnabled(gateway_id) => {
                self.draft.toggle_gateway(gateway_id);
                MachineOutcome::Continue
            }
            MenuAction::EditGatewayToken(gateway_id) => {
                let current = find_gateway(gateway_id)
                    .and_then(|g| self.draft.pending_secrets.get(g.vault_secret_key))
                    .cloned()
                    .unwrap_or_default();
                self.stack.push(Screen::GatewayTokenInput {
                    gateway_id,
                    buffer: current,
                });
                MachineOutcome::Continue
            }
            MenuAction::GoBack => self.go_back(),
        }
    }

    fn confirm_input(&mut self) -> MachineOutcome {
        let Some(top) = self.stack.pop() else {
            return MachineOutcome::Continue;
        };
        match top {
            Screen::ProviderInput {
                provider_id,
                step_idx,
                buffer,
            } => {
                let desc = find_provider(provider_id);
                let trimmed = buffer.trim().to_string();
                match PROVIDER_FLOW_STEPS[step_idx] {
                    ProviderField::BaseUrl => {
                        let switched_provider = !self
                            .draft
                            .config
                            .provider
                            .kind
                            .eq_ignore_ascii_case(desc.id);
                        self.draft.config.provider.kind = desc.id.to_string();
                        self.draft.config.provider.base_url = match trimmed.is_empty() {
                            true => desc.default_base_url.to_string(),
                            false => trimmed,
                        };
                        if switched_provider {
                            self.draft.config.provider.lead_model.clear();
                            self.draft.config.provider.fast_model.clear();
                            self.draft.config.provider.compaction_model.clear();
                            self.draft.config.provider.dream_model.clear();
                            self.draft.config.provider.fallbacks.clear();
                        }
                    }
                    ProviderField::ApiKey => {
                        if !trimmed.is_empty() {
                            self.draft
                                .pending_secrets
                                .insert(desc.vault_secret_key.to_string(), trimmed);
                        }
                    }
                    ProviderField::LeadModel => {
                        self.draft.config.provider.lead_model = trimmed;
                    }
                }

                let next_idx = step_idx + 1;
                match PROVIDER_FLOW_STEPS.get(next_idx) {
                    Some(next_field) => {
                        let next_buf = self.initial_provider_field_value(provider_id, *next_field);
                        self.stack.push(Screen::ProviderInput {
                            provider_id,
                            step_idx: next_idx,
                            buffer: next_buf,
                        });
                    }
                    None => {
                        self.draft.status_notice =
                            Some(format!("Configured provider: {}", desc.label));
                        // Pop Providers list too so user lands right back on Root
                        if matches!(self.stack.last(), Some(Screen::Providers { .. })) {
                            self.stack.pop();
                        }
                    }
                }
                MachineOutcome::Continue
            }
            Screen::GatewayTokenInput { gateway_id, buffer } => {
                if let Some(gw) = find_gateway(gateway_id) {
                    let trimmed = buffer.trim().to_string();
                    if !trimmed.is_empty() {
                        self.draft
                            .pending_secrets
                            .insert(gw.vault_secret_key.to_string(), trimmed);
                        if !self.draft.gateway_enabled(gateway_id) {
                            self.draft.toggle_gateway(gateway_id);
                        }
                        self.draft.status_notice =
                            Some(format!("Updated {} secret & enabled gateway", gw.label));
                    }
                }
                MachineOutcome::Continue
            }
            other => {
                self.stack.push(other);
                MachineOutcome::Continue
            }
        }
    }

    fn initial_provider_field_value(&self, provider_id: &'static str, field: ProviderField) -> String {
        let desc = find_provider(provider_id);
        let is_same = self.draft.config.provider.kind.eq_ignore_ascii_case(provider_id);
        match field {
            ProviderField::BaseUrl => match is_same && !self.draft.config.provider.base_url.is_empty() {
                true => self.draft.config.provider.base_url.clone(),
                false => desc.default_base_url.to_string(),
            },
            ProviderField::ApiKey => self
                .draft
                .pending_secrets
                .get(desc.vault_secret_key)
                .cloned()
                .unwrap_or_default(),
            ProviderField::LeadModel => match is_same {
                true => self.draft.config.provider.lead_model.clone(),
                false => String::new(),
            },
        }
    }

    fn view_root(&self, cursor: usize) -> ScreenView {
        let active_provider = find_provider(&self.draft.config.provider.kind);
        let provider_ready =
            !active_provider.requires_api_key || self.draft.has_secret(active_provider.vault_secret_key);
        let stream_count = STREAMING_REGISTRY
            .iter()
            .filter(|o| self.draft.streaming_value(o.id))
            .count();
        let active_gateways = GATEWAY_REGISTRY
            .iter()
            .filter(|g| self.draft.gateway_enabled(g.id))
            .count();

        let items = vec![
            MenuEntry {
                label: "Providers".into(),
                detail: format!(
                    "{} · {}",
                    active_provider.label,
                    match self.draft.config.provider.lead_model.is_empty() {
                        true => "auto-discover best free/working",
                        false => self.draft.config.provider.lead_model.as_str(),
                    }
                ),
                badge: match provider_ready {
                    true => "READY".into(),
                    false => "KEY NEEDED".into(),
                },
                badge_active: provider_ready,
                action: MenuAction::OpenProviders,
            },
            MenuEntry {
                label: "Streaming".into(),
                detail: "SSE streaming, KV prompt caching & live drafts".into(),
                badge: format!("{stream_count}/{} ON", STREAMING_REGISTRY.len()),
                badge_active: stream_count > 0,
                action: MenuAction::OpenStreaming,
            },
            MenuEntry {
                label: "Gateway".into(),
                detail: "Telegram bot, pairing codes & signed webhooks".into(),
                badge: match active_gateways {
                    0 => "OPTIONAL".into(),
                    n => format!("{n} ACTIVE"),
                },
                badge_active: active_gateways > 0,
                action: MenuAction::OpenGateways,
            },
            MenuEntry {
                label: "Save & Finish".into(),
                detail: "Encrypt secrets into vault & write zenko.toml".into(),
                badge: "ENTER".into(),
                badge_active: true,
                action: MenuAction::SaveAndFinish,
            },
        ];

        ScreenView::Menu {
            title: "Required Setup".into(),
            subtitle: "Select a section with ↑/↓ and press Enter".into(),
            cursor,
            items,
        }
    }

    fn view_providers(&self, cursor: usize) -> ScreenView {
        let mut items: Vec<MenuEntry> = PROVIDER_REGISTRY
            .iter()
            .map(|p| {
                let is_selected = self.draft.config.provider.kind.eq_ignore_ascii_case(p.id);
                let has_key = self.draft.has_secret(p.vault_secret_key);
                let badge = match (is_selected, p.requires_api_key, has_key) {
                    (true, _, true) => "ACTIVE · KEY SET".into(),
                    (true, false, _) => "ACTIVE · LOCAL".into(),
                    (true, true, false) => "ACTIVE".into(),
                    (false, false, _) => "LOCAL".into(),
                    (false, true, true) => "KEY SET".into(),
                    (false, true, false) => "AVAILABLE".into(),
                };
                MenuEntry {
                    label: p.label.to_string(),
                    detail: p.description.to_string(),
                    badge,
                    badge_active: is_selected || has_key,
                    action: MenuAction::SelectProvider(p.id),
                }
            })
            .collect();

        items.push(go_back_entry());

        ScreenView::Menu {
            title: "Select Provider".into(),
            subtitle: "Choose an LLM provider to configure Base URL, API Key & Model".into(),
            cursor,
            items,
        }
    }

    fn view_provider_input(
        &self,
        provider_id: &'static str,
        step_idx: usize,
        buffer: &str,
    ) -> ScreenView {
        let desc = find_provider(provider_id);
        let field = PROVIDER_FLOW_STEPS[step_idx];
        let step_label = format!("Step {} of {}", step_idx + 1, PROVIDER_FLOW_STEPS.len());

        let (title, subtitle, masked, placeholder) = match field {
            ProviderField::BaseUrl => (
                format!("{} — Base URL", desc.label),
                "Press Enter to keep default or edit endpoint URL".into(),
                false,
                desc.default_base_url.to_string(),
            ),
            ProviderField::ApiKey => {
                let stored = self.draft.has_secret(desc.vault_secret_key);
                let sub = match (desc.requires_api_key, stored) {
                    (false, _) => "Optional for local endpoint — press Enter to skip".into(),
                    (true, true) => {
                        "Key already stored in vault — paste new key or press Enter to keep".into()
                    }
                    (true, false) => {
                        "Paste your API key (stored encrypted in ~/.zenko/vault.enc)".into()
                    }
                };
                (
                    format!("{} — API Key", desc.label),
                    sub,
                    true,
                    "Paste secret key...".into(),
                )
            }
            ProviderField::LeadModel => (
                format!("{} — Default Model", desc.label),
                "Optional model ID — leave blank to auto-discover top free/working model".into(),
                false,
                "auto-discover from provider".into(),
            ),
        };

        ScreenView::Input(InputPromptView {
            title,
            subtitle,
            value: buffer.to_string(),
            masked,
            step_label,
            placeholder,
        })
    }

    fn view_streaming(&self, cursor: usize) -> ScreenView {
        let mut items: Vec<MenuEntry> = STREAMING_REGISTRY
            .iter()
            .map(|opt| {
                let on = self.draft.streaming_value(opt.id);
                MenuEntry {
                    label: opt.label.to_string(),
                    detail: opt.description.to_string(),
                    badge: match on {
                        true => "ON".into(),
                        false => "OFF".into(),
                    },
                    badge_active: on,
                    action: MenuAction::ToggleStreaming(opt.id),
                }
            })
            .collect();

        items.push(go_back_entry());

        ScreenView::Menu {
            title: "Streaming & Caching".into(),
            subtitle: "Press Enter or Space to toggle each streaming feature".into(),
            cursor,
            items,
        }
    }

    fn view_gateways(&self, cursor: usize) -> ScreenView {
        let mut items: Vec<MenuEntry> = GATEWAY_REGISTRY
            .iter()
            .map(|gw| {
                let enabled = self.draft.gateway_enabled(gw.id);
                let has_tok = self.draft.has_secret(gw.vault_secret_key);
                let badge = match (enabled, has_tok) {
                    (true, true) => "ENABLED · TOKEN SET".into(),
                    (true, false) => "ENABLED".into(),
                    (false, true) => "TOKEN SET".into(),
                    (false, false) => "OFF".into(),
                };
                MenuEntry {
                    label: gw.label.to_string(),
                    detail: gw.description.to_string(),
                    badge,
                    badge_active: enabled,
                    action: MenuAction::OpenGateway(gw.id),
                }
            })
            .collect();

        items.push(go_back_entry());

        ScreenView::Menu {
            title: "Select Gateway".into(),
            subtitle: "Connect messaging channels to your 24/7 Zenko daemon".into(),
            cursor,
            items,
        }
    }

    fn view_gateway_menu(&self, gateway_id: &'static str, cursor: usize) -> ScreenView {
        let gw = find_gateway(gateway_id).unwrap_or(&GATEWAY_REGISTRY[0]);
        let enabled = self.draft.gateway_enabled(gw.id);
        let has_tok = self.draft.has_secret(gw.vault_secret_key);

        let items = vec![
            MenuEntry {
                label: format!("Enable {}", gw.label),
                detail: "Start this gateway automatically with `zenko web`".into(),
                badge: match enabled {
                    true => "ON".into(),
                    false => "OFF".into(),
                },
                badge_active: enabled,
                action: MenuAction::ToggleGatewayEnabled(gw.id),
            },
            MenuEntry {
                label: "Bot Token / Secret".into(),
                detail: gw.token_prompt.to_string(),
                badge: match has_tok {
                    true => "STORED IN VAULT".into(),
                    false => "NOT SET".into(),
                },
                badge_active: has_tok,
                action: MenuAction::EditGatewayToken(gw.id),
            },
            go_back_entry(),
        ];

        ScreenView::Menu {
            title: format!("{} Gateway", gw.label),
            subtitle: gw.description.to_string(),
            cursor,
            items,
        }
    }

    fn view_gateway_token_input(&self, gateway_id: &'static str, buffer: &str) -> ScreenView {
        let gw = find_gateway(gateway_id).unwrap_or(&GATEWAY_REGISTRY[0]);
        let stored = self.draft.has_secret(gw.vault_secret_key);
        let subtitle = match stored {
            true => "Token already in vault — paste a replacement or press Enter to keep".into(),
            false => format!("{} (encrypted in ~/.zenko/vault.enc)", gw.token_prompt),
        };
        ScreenView::Input(InputPromptView {
            title: format!("{} — Token / Secret", gw.label),
            subtitle,
            value: buffer.to_string(),
            masked: true,
            step_label: "Secret Input".into(),
            placeholder: "Paste token...".into(),
        })
    }
}

fn go_back_entry() -> MenuEntry {
    MenuEntry {
        label: "← Go Back".into(),
        detail: "Return to previous menu (or press Esc)".into(),
        badge: "ESC".into(),
        badge_active: false,
        action: MenuAction::GoBack,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_stack_navigates_providers_streaming_gateway_and_back() {
        let mut m = SetupMachine::new(ZenkoConfig::default(), BTreeMap::new());
        // Enter Providers
        assert_eq!(m.confirm(), MachineOutcome::Continue);
        assert_eq!(m.breadcrumbs(), vec!["Setup", "Providers"]);

        // Select OpenRouter -> Base URL -> ApiKey -> LeadModel
        assert_eq!(m.confirm(), MachineOutcome::Continue);
        assert!(m.is_input_mode());
        // Go back from Base URL returns to Providers
        assert_eq!(m.go_back(), MachineOutcome::Continue);
        assert_eq!(m.breadcrumbs(), vec!["Setup", "Providers"]);

        // Select OpenRouter again and walk all 3 steps
        assert_eq!(m.confirm(), MachineOutcome::Continue); // BaseUrl
        assert_eq!(m.confirm(), MachineOutcome::Continue); // ApiKey
        m.paste_text("sk-or-test-key");
        assert_eq!(m.confirm(), MachineOutcome::Continue); // LeadModel
        assert_eq!(m.confirm(), MachineOutcome::Continue); // commits & pops to Root
        assert_eq!(m.breadcrumbs(), vec!["Setup"]);
        assert!(m.draft.has_secret("OPENROUTER_API_KEY"));

        // Switch provider to MiniMax (second entry in PROVIDER_REGISTRY)
        assert_eq!(m.confirm(), MachineOutcome::Continue); // Providers
        m.move_cursor(1); // MiniMax
        assert_eq!(m.confirm(), MachineOutcome::Continue); // BaseUrl
        assert_eq!(m.confirm(), MachineOutcome::Continue); // ApiKey
        m.paste_text("mm-test-secret");
        assert_eq!(m.confirm(), MachineOutcome::Continue); // LeadModel
        assert_eq!(m.confirm(), MachineOutcome::Continue); // commits & pops to Root
        assert_eq!(m.draft.config.provider.kind, "minimax");
        assert_eq!(m.draft.config.provider.base_url, "https://api.minimax.io/v1");
        assert!(m.draft.config.provider.lead_model.is_empty());
        assert!(m.draft.has_secret("MINIMAX_API_KEY"));

        // Navigate down to Streaming and toggle option 0
        m.move_cursor(1);
        assert_eq!(m.confirm(), MachineOutcome::Continue);
        assert_eq!(m.breadcrumbs(), vec!["Setup", "Streaming"]);
        assert!(m.draft.config.provider.enable_streaming);
        assert_eq!(m.confirm(), MachineOutcome::Continue);
        assert!(!m.draft.config.provider.enable_streaming);

        // Esc goes back to Root
        assert_eq!(m.go_back(), MachineOutcome::Continue);
        assert_eq!(m.breadcrumbs(), vec!["Setup"]);

        // Navigate to Gateway -> Telegram -> Back
        m.move_cursor(1);
        assert_eq!(m.confirm(), MachineOutcome::Continue);
        assert_eq!(m.breadcrumbs(), vec!["Setup", "Gateway"]);
        assert_eq!(m.confirm(), MachineOutcome::Continue);
        assert_eq!(m.breadcrumbs(), vec!["Setup", "Gateway", "Telegram"]);
        assert_eq!(m.confirm(), MachineOutcome::Continue);
        assert!(m.draft.config.telegram.enabled);
    }
}

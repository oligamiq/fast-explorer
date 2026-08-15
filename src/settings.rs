use serde::{Deserialize, Serialize};

use crate::theme::{ThemePatch, ThemeSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    #[default]
    Default,
    Everything,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UiFont {
    #[default]
    System,
    Sans,
    Serif,
    Monospace,
    Rounded,
}

impl UiFont {
    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Sans => "Sans",
            Self::Serif => "Serif",
            Self::Monospace => "Monospace",
            Self::Rounded => "Rounded",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteCacheSettings {
    #[serde(default = "default_remote_cache_limit_mib")]
    pub limit_mib: u32,
    #[serde(default = "default_remote_cache_expiration_hours")]
    pub expiration_hours: u32,
}

const fn default_remote_cache_limit_mib() -> u32 {
    1024
}

const fn default_remote_cache_expiration_hours() -> u32 {
    24
}

const fn default_confirm_mobile_delete() -> bool {
    true
}

impl Default for RemoteCacheSettings {
    fn default() -> Self {
        Self {
            limit_mib: default_remote_cache_limit_mib(),
            expiration_hours: default_remote_cache_expiration_hours(),
        }
    }
}

impl SearchMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "default" | "native" => Some(Self::Default),
            "everything" | "es" => Some(Self::Everything),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Everything => "Everything",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailnetProfileSettings {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub enabled: bool,
}

impl TailnetProfileSettings {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            enabled: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(flatten)]
    pub theme: ThemeSettings,
    #[serde(default)]
    pub search_mode: SearchMode,
    #[serde(default)]
    pub ui_font: UiFont,
    #[serde(default)]
    pub remote_cache: RemoteCacheSettings,
    #[serde(default)]
    pub tailscale_profiles: Vec<TailnetProfileSettings>,
    #[serde(default)]
    pub pinned_paths: Vec<std::path::PathBuf>,
    #[serde(default = "default_confirm_mobile_delete")]
    pub confirm_mobile_delete: bool,
    #[serde(default)]
    pub delete_warning_suppressed_until_ms: u64,
    #[serde(default, rename = "tailscale_enabled", skip_serializing)]
    legacy_tailscale_enabled: bool,
}

impl AppSettings {
    pub fn new(
        theme: ThemeSettings,
        search_mode: SearchMode,
        ui_font: UiFont,
        remote_cache: RemoteCacheSettings,
        tailscale_profiles: Vec<TailnetProfileSettings>,
    ) -> Self {
        Self {
            theme,
            search_mode,
            ui_font,
            remote_cache,
            tailscale_profiles,
            pinned_paths: Vec::new(),
            confirm_mobile_delete: true,
            delete_warning_suppressed_until_ms: 0,
            legacy_tailscale_enabled: false,
        }
    }

    pub fn migrate_legacy(mut self) -> Self {
        if self.tailscale_profiles.is_empty() && self.legacy_tailscale_enabled {
            let mut profile = TailnetProfileSettings::new("tailnet-1", "Tailnet 1");
            profile.enabled = true;
            self.tailscale_profiles.push(profile);
        }
        self.legacy_tailscale_enabled = false;
        self
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: ThemeSettings::default(),
            search_mode: SearchMode::Default,
            ui_font: UiFont::System,
            remote_cache: RemoteCacheSettings::default(),
            tailscale_profiles: Vec::new(),
            pinned_paths: Vec::new(),
            confirm_mobile_delete: true,
            delete_warning_suppressed_until_ms: 0,
            legacy_tailscale_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSettingsPatch {
    #[serde(flatten)]
    pub theme: ThemePatch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_mode: Option<SearchMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_font: Option<UiFont>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_cache: Option<RemoteCacheSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale_profiles: Option<Vec<TailnetProfileSettings>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale_enabled: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{AppearanceMode, ThemeColor};

    #[test]
    fn old_flat_theme_config_defaults_search_mode() {
        let settings = serde_json::from_str::<AppSettings>(
            r#"{"appearance":"dark","color":"red","intensity":80}"#,
        )
        .expect("legacy flat config")
        .migrate_legacy();
        assert_eq!(settings.theme.appearance, AppearanceMode::Dark);
        assert_eq!(settings.theme.color, ThemeColor::Red);
        assert_eq!(settings.theme.intensity, 80);
        assert_eq!(settings.search_mode, SearchMode::Default);
        assert_eq!(settings.ui_font, UiFont::System);
        assert_eq!(settings.remote_cache, RemoteCacheSettings::default());
        assert!(settings.tailscale_profiles.is_empty());
        assert!(settings.confirm_mobile_delete);
        assert_eq!(settings.delete_warning_suppressed_until_ms, 0);
    }

    #[test]
    fn legacy_tailscale_flag_migrates_to_default_profile() {
        let settings = serde_json::from_str::<AppSettings>(
            r#"{"appearance":"system","color":"blue","intensity":72,"tailscale_enabled":true}"#,
        )
        .expect("legacy tailscale config")
        .migrate_legacy();
        assert_eq!(settings.tailscale_profiles.len(), 1);
        assert_eq!(settings.tailscale_profiles[0].id, "tailnet-1");
        assert_eq!(settings.tailscale_profiles[0].label, "Tailnet 1");
        assert!(settings.tailscale_profiles[0].enabled);
    }

    #[test]
    fn profiles_serialize_and_round_trip() {
        let mut profile = TailnetProfileSettings::new("work", "Work");
        profile.enabled = true;
        let settings = AppSettings::new(
            ThemeSettings::default(),
            SearchMode::Everything,
            UiFont::Monospace,
            RemoteCacheSettings {
                limit_mib: 2048,
                expiration_hours: 72,
            },
            vec![profile],
        );
        let json = serde_json::to_string(&settings).expect("serialize settings");
        assert!(!json.contains("tailscale_enabled"));
        assert!(json.contains("tailscale_profiles"));
        assert!(json.contains("\"ui_font\":\"monospace\""));
        assert_eq!(
            serde_json::from_str::<AppSettings>(&json).unwrap(),
            settings
        );
    }
}

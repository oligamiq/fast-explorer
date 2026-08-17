use std::collections::{BTreeMap, BTreeSet};

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
pub enum PathOverflowBehavior {
    Static,
    #[default]
    ForwardReset,
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

const fn default_path_reset_delay_ms() -> u32 {
    3000
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
    pub path: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub device_names: BTreeMap<String, String>,
    #[serde(default)]
    pub enabled: bool,
}

impl TailnetProfileSettings {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            path: String::new(),
            hostname: String::new(),
            device_names: BTreeMap::new(),
            enabled: false,
        }
    }
}

pub fn automatic_tailnet_path(profile_id: &str) -> String {
    let mut hash = 0x811c9dc5_u32;
    for byte in profile_id.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x01000193);
    }
    format!("tn-{:06x}", hash & 0x00ff_ffff)
}

pub fn normalize_tailnet_path(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len().min(16));
    let mut previous_dash = false;
    for ch in value.trim().chars() {
        let next = if ch.is_alphanumeric() || ch == '_' {
            ch
        } else {
            '-'
        };
        if next == '-' {
            if previous_dash || normalized.is_empty() {
                continue;
            }
            previous_dash = true;
        } else {
            previous_dash = false;
        }
        normalized.push(next);
        if normalized.chars().count() >= 16 {
            break;
        }
    }
    while normalized.ends_with('-') {
        normalized.pop();
    }
    normalized
}

pub fn normalize_taildrive_device_name(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len().min(24));
    let mut previous_space = false;
    for ch in value.trim().chars() {
        let next = if matches!(ch, '/' | '\\') { '-' } else { ch };
        if next.is_whitespace() {
            if previous_space || normalized.is_empty() {
                continue;
            }
            normalized.push(' ');
            previous_space = true;
        } else {
            normalized.push(next);
            previous_space = false;
        }
        if normalized.chars().count() >= 24 {
            break;
        }
    }
    let normalized = normalized.trim_end().to_owned();
    if matches!(normalized.as_str(), "." | "..") {
        String::new()
    } else {
        normalized
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SupabaseSettings {
    #[serde(default)]
    pub project_url: String,
    #[serde(default)]
    pub publishable_key: String,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub email: String,
}

impl SupabaseSettings {
    pub fn is_configured(&self) -> bool {
        !self.project_url.trim().is_empty() && !self.publishable_key.trim().is_empty()
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
    pub path_overflow_behavior: PathOverflowBehavior,
    #[serde(default = "default_path_reset_delay_ms")]
    pub path_reset_delay_ms: u32,
    #[serde(default)]
    pub tailscale_profiles: Vec<TailnetProfileSettings>,
    #[serde(default)]
    pub supabase: SupabaseSettings,
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
            path_overflow_behavior: PathOverflowBehavior::default(),
            path_reset_delay_ms: default_path_reset_delay_ms(),
            tailscale_profiles,
            supabase: SupabaseSettings::default(),
            pinned_paths: Vec::new(),
            confirm_mobile_delete: true,
            delete_warning_suppressed_until_ms: 0,
            legacy_tailscale_enabled: false,
        }
    }

    pub fn migrate_legacy(mut self) -> Self {
        if self.tailscale_profiles.is_empty() && self.legacy_tailscale_enabled {
            let mut profile = TailnetProfileSettings::new("tailnet-1", "TN1");
            profile.enabled = true;
            self.tailscale_profiles.push(profile);
        }

        // Older FastExplorer builds generated long Tailscale hostnames and
        // equally long default labels. Treat only our known autogenerated
        // values as migratable so user-chosen names are never overwritten.
        let mut used_paths = BTreeSet::new();
        let mut next_default = 1usize;
        for profile in &mut self.tailscale_profiles {
            let hostname = profile.hostname.trim().to_ascii_lowercase();
            if hostname.starts_with("fastexplorer-") || hostname.starts_with("fast-explorer-") {
                profile.hostname.clear();
            }
            if let Some(number) = profile.label.strip_prefix("Tailnet ")
                && !number.is_empty()
                && number.chars().all(|ch| ch.is_ascii_digit())
            {
                profile.label = format!("TN{number}");
            }
            profile.device_names.retain(|_, name| {
                *name = normalize_taildrive_device_name(name);
                !name.is_empty()
            });

            let requested = normalize_tailnet_path(&profile.path);
            let mut effective = if requested.is_empty() {
                automatic_tailnet_path(&profile.id)
            } else {
                requested.clone()
            };
            if used_paths.contains(&effective.to_lowercase()) {
                if requested.is_empty() {
                    loop {
                        let candidate = format!("tn{next_default}");
                        next_default += 1;
                        if !used_paths.contains(&candidate.to_lowercase()) {
                            profile.path = candidate.clone();
                            effective = candidate;
                            break;
                        }
                    }
                } else {
                    let base = requested;
                    let mut suffix = 2usize;
                    loop {
                        let candidate = format!("{base}-{suffix}");
                        suffix += 1;
                        if !used_paths.contains(&candidate.to_lowercase()) {
                            profile.path = candidate.clone();
                            effective = candidate;
                            break;
                        }
                    }
                }
            } else {
                profile.path = requested;
            }
            used_paths.insert(effective.to_lowercase());
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
            path_overflow_behavior: PathOverflowBehavior::default(),
            path_reset_delay_ms: default_path_reset_delay_ms(),
            tailscale_profiles: Vec::new(),
            supabase: SupabaseSettings::default(),
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
    pub path_overflow_behavior: Option<PathOverflowBehavior>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_reset_delay_ms: Option<u32>,
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
        assert_eq!(
            settings.path_overflow_behavior,
            PathOverflowBehavior::ForwardReset
        );
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
        assert_eq!(settings.tailscale_profiles[0].label, "TN1");
        assert!(settings.tailscale_profiles[0].path.is_empty());
        assert!(automatic_tailnet_path(&settings.tailscale_profiles[0].id).starts_with("tn-"));
        assert!(settings.tailscale_profiles[0].enabled);
    }

    #[test]
    fn autogenerated_tailnet_names_are_migrated_without_touching_custom_names() {
        let settings = serde_json::from_str::<AppSettings>(
            r#"{"tailscale_profiles":[{"id":"old","label":"Tailnet 3","hostname":"fastexplorer-very-long-generated-name","enabled":true},{"id":"custom","label":"Office","hostname":"office-pc","enabled":true}]}"#,
        )
        .expect("legacy profiles")
        .migrate_legacy();
        assert_eq!(settings.tailscale_profiles[0].label, "TN3");
        assert!(settings.tailscale_profiles[0].path.is_empty());
        assert!(settings.tailscale_profiles[0].hostname.is_empty());
        assert_eq!(settings.tailscale_profiles[1].label, "Office");
        assert!(settings.tailscale_profiles[1].path.is_empty());
        assert_ne!(
            automatic_tailnet_path(&settings.tailscale_profiles[0].id),
            automatic_tailnet_path(&settings.tailscale_profiles[1].id)
        );
        assert_eq!(settings.tailscale_profiles[1].hostname, "office-pc");
    }

    #[test]
    fn taildrive_short_path_components_are_sanitized_and_bounded() {
        assert_eq!(normalize_tailnet_path(" ../ Work / Team "), "Work-Team");
        assert_eq!(
            normalize_taildrive_device_name(" desk/phone "),
            "desk-phone"
        );
        assert!(normalize_taildrive_device_name("..").is_empty());
        assert_eq!(
            normalize_taildrive_device_name("abcdefghijklmnopqrstuvwxyz")
                .chars()
                .count(),
            24
        );
    }

    #[test]
    fn duplicate_tailnet_paths_are_migrated_to_unique_short_paths() {
        let settings = serde_json::from_str::<AppSettings>(
            r#"{"tailscale_profiles":[{"id":"one","label":"One","path":"work","enabled":true},{"id":"two","label":"Two","path":"WORK","enabled":true},{"id":"three","label":"Three","path":"","enabled":true}]}"#,
        )
        .expect("profiles")
        .migrate_legacy();
        assert_eq!(settings.tailscale_profiles[0].path, "work");
        assert_eq!(settings.tailscale_profiles[1].path, "WORK-2");
        assert!(settings.tailscale_profiles[2].path.is_empty());
        assert_eq!(
            automatic_tailnet_path(&settings.tailscale_profiles[2].id),
            automatic_tailnet_path("three")
        );
    }

    #[test]
    fn automatic_tailnet_path_survives_save_and_reload() {
        let mut profile = TailnetProfileSettings::new("stable-profile-id", "Work");
        profile.path.clear();
        let settings = AppSettings::new(
            ThemeSettings::default(),
            SearchMode::Default,
            UiFont::System,
            RemoteCacheSettings::default(),
            vec![profile],
        );
        let before = automatic_tailnet_path("stable-profile-id");
        let json = serde_json::to_string(&settings).expect("serialize settings");
        let restored = serde_json::from_str::<AppSettings>(&json)
            .expect("deserialize settings")
            .migrate_legacy();
        assert!(restored.tailscale_profiles[0].path.is_empty());
        assert_eq!(
            automatic_tailnet_path(&restored.tailscale_profiles[0].id),
            before
        );
    }

    #[test]
    fn profiles_serialize_and_round_trip() {
        let mut profile = TailnetProfileSettings::new("work", "Work");
        profile.path = "w".to_owned();
        profile
            .device_names
            .insert("node-1".to_owned(), "desk".to_owned());
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

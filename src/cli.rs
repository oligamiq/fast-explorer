use std::path::PathBuf;

use crate::settings::SearchMode;
use crate::theme::{AppearanceMode, ThemeColor, ThemePatch};

#[derive(Debug, Clone, Default)]
pub struct CliOptions {
    pub theme_overrides: ThemePatch,
    pub search_override: Option<SearchMode>,
    pub ipc_socket: Option<PathBuf>,
    pub ipc_enabled: bool,
    pub startup_path: Option<PathBuf>,
    pub show_help: bool,
}

impl CliOptions {
    pub fn parse() -> Result<Self, String> {
        let mut options = Self {
            ipc_enabled: true,
            ..Self::default()
        };
        let args = std::env::args().skip(1).collect::<Vec<_>>();
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            if arg == "--help" || arg == "-h" {
                options.show_help = true;
                index += 1;
                continue;
            }
            if arg == "--no-ipc" {
                options.ipc_enabled = false;
                index += 1;
                continue;
            }

            if !arg.starts_with('-') {
                if options.startup_path.is_some() {
                    return Err(format!("multiple startup paths are not supported: {arg}"));
                }
                options.startup_path = Some(PathBuf::from(arg));
                index += 1;
                continue;
            }

            let (key, inline_value) = arg
                .split_once('=')
                .map_or((arg.as_str(), None), |(key, value)| (key, Some(value)));
            let value = match key {
                "--appearance" | "--theme-color" | "--color" | "--theme-intensity"
                | "--intensity" | "--search-mode" | "--ipc-socket" => {
                    if let Some(value) = inline_value {
                        value.to_owned()
                    } else {
                        index += 1;
                        args.get(index)
                            .cloned()
                            .ok_or_else(|| format!("missing value for {key}"))?
                    }
                }
                _ => return Err(format!("unknown argument: {arg}")),
            };

            match key {
                "--appearance" => {
                    options.theme_overrides.appearance = Some(
                        AppearanceMode::parse(&value)
                            .ok_or_else(|| format!("invalid appearance: {value}"))?,
                    );
                }
                "--theme-color" | "--color" => {
                    options.theme_overrides.color = Some(
                        ThemeColor::parse(&value)
                            .ok_or_else(|| format!("invalid theme color: {value}"))?,
                    );
                }
                "--theme-intensity" | "--intensity" => {
                    let intensity = value
                        .parse::<u8>()
                        .map_err(|_| format!("invalid intensity: {value}"))?;
                    if intensity > 100 {
                        return Err(format!("intensity must be 0..=100, got {intensity}"));
                    }
                    options.theme_overrides.intensity = Some(intensity);
                }
                "--search-mode" => {
                    options.search_override = Some(
                        SearchMode::parse(&value)
                            .ok_or_else(|| format!("invalid search mode: {value}"))?,
                    );
                }
                "--ipc-socket" => options.ipc_socket = Some(PathBuf::from(value)),
                _ => unreachable!(),
            }
            index += 1;
        }
        Ok(options)
    }

    pub const HELP: &'static str = "FastExplorer [path]\n\
  path                                open this folder at startup\n\
  --appearance <system|light|dark>   temporary appearance override\n\
  --theme-color <name>               temporary color override\n\
  --theme-intensity <0..100>         temporary theme intensity override\n\
  --search-mode <default|everything> temporary search backend override\n\
  --ipc-socket <path>                override local IPC socket path\n\
  --no-ipc                            disable local IPC server\n";
}

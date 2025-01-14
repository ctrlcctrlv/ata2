//! Configuration file parsing and validation.
//!
//! # ata²
//!
//!	 © 2023    Fredrick R. Brennan <copypaste@kittens.ph>
//!	 © 2023    Rik Huijzer <t.h.huijzer@rug.nl>
//!	 © 2023–   ATA Project Authors
//!
//!  Licensed under the Apache License, Version 2.0 (the "License");
//!  you may _not_ use this file except in compliance with the License.
//!  You may obtain a copy of the License at
//!
//!      http://www.apache.org/licenses/LICENSE-2.0
//!
//!  Unless required by applicable law or agreed to in writing, software
//!  distributed under the License is distributed on an "AS IS" BASIS,
//!  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//!  See the License for the specific language governing permissions and
//!  limitations under the License.

use std::collections::HashMap as StdHashMap;
use std::convert::Infallible;
use std::env;
use std::ffi::OsString;
use std::fmt::{self, Display};

use std::path::{Path, PathBuf};
use std::str::FromStr;

use ansi_colors::ColouredStr;
use async_openai::{config::OpenAIConfig, types::CreateChatCompletionRequestArgs};
use bevy_reflect::{Reflect, Struct};
use bevy_utils::HashMap;
use directories::ProjectDirs;
use os_str_bytes::OsStrBytes as _;
use os_str_bytes::OsStringBytes as _;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use toml::de::Error as TomlError;

lazy_static! {
    pub(crate) static ref DEFAULT_CONFIG_FILENAME: PathBuf = "ata2.toml".into();
    pub(crate) static ref DEFAULT_CONFIG_FILENAME_V1: PathBuf = "ata.toml".into();
}

/// UI config
#[repr(C)]
#[derive(Clone, Deserialize, Debug, Serialize, Reflect)]
#[serde(default)]
pub struct UiConfig {
    /// Require user to press ^C twice?
    pub double_ctrlc: bool,
    /// Hide config on run?
    pub hide_config: bool,
    /// Redact API key?
    pub redact_api_key: bool,
    /// Allow multiline insertions? If so, you end the input by sending an EOF (^D).
    pub multiline_insertions: bool,
    /// Save history?
    pub save_history: bool,
    /// History file
    pub history_file: PathBuf,
}

/// For definitions, see <https://platform.openai.com/docs/api-reference/completions/create>.
#[repr(C)]
#[derive(Clone, Deserialize, Debug, Serialize, Reflect)]
#[serde(default)]
pub struct Config {
    pub api_key: Option<String>,
    pub model: String,
    pub max_tokens: i64,
    pub temperature: f64,
    pub suffix: Option<String>,
    pub top_p: f64,
    pub n: u64,
    pub stream: bool,
    pub stop: Vec<String>,
    pub presence_penalty: f64,
    pub frequency_penalty: f64,
    pub logit_bias: HashMap<String, f64>,
    pub user_id: Option<String>,
    pub ui: UiConfig,
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        match self.api_key.as_ref().map(|s| s.as_str()) {
            Some("") | None => return Err(String::from("API key is missing")),
            _ => {}
        }

        if self.model.is_empty() {
            return Err(String::from("Model ID is missing"));
        }

        if self.max_tokens < 1 || self.max_tokens > 2048 {
            return Err(String::from("Max tokens must be between 1 and 2048"));
        }

        if self.temperature < 0.0 || self.temperature > 1.0 {
            return Err(String::from("Temperature must be between 0.0 and 1.0"));
        }

        if let Some(suffix) = &self.suffix {
            if suffix.is_empty() {
                return Err(String::from("Suffix cannot be an empty string"));
            }
        }

        if self.top_p < 0.0 || self.top_p > 1.0 {
            return Err(String::from("Top-p must be between 0.0 and 1.0"));
        }

        if self.n < 1 || self.n > 10 {
            return Err(String::from("n must be between 1 and 10"));
        }

        if self.stop.iter().any(|stop| stop.is_empty()) || self.stop.len() > 4 {
            return Err(String::from("Stop phrases cannot contain empties"));
        }

        if self.presence_penalty < 0.0 || self.presence_penalty > 1.0 {
            return Err(String::from("Presence penalty must be between 0.0 and 1.0"));
        }

        if self.frequency_penalty < 0.0 || self.frequency_penalty > 1.0 {
            return Err(String::from(
                "Frequency penalty must be between 0.0 and 1.0",
            ));
        }

        match self.user_id.as_ref().map(|s| s.as_str()) {
            Some("") => return Err(String::from("User ID cannot be an empty string")),
            _ => {}
        }

        for (key, value) in &self.logit_bias {
            if value < &-2.0 || value > &2.0 {
                return Err(format!(
                    "logit_bias for {} must be between -2.0 and 2.0",
                    key
                ));
            }
        }

        Ok(self.ui.validate()?)
    }
}

/// Note: the result is heavily based on the environment variables.
///
/// * `ATA2_MODEL` sets the model ID. Default: `gpt-3.5-turbo`.
/// * `ATA2_MAX_TOKENS` sets the maximum amount of tokens that the server can answer with. Longer answers will be truncated. Default: `2048`.
/// * `ATA2_TEMPERATURE`. Default: `0.8`.
/// * `ATA2_SUFFIX` sets the suffix. Default: `None`.
/// * `ATA2_TOP_P`. Default: `1.0`.
/// * `ATA2_N`. Default: `1`.
/// * `ATA2_STOP` sets the stop phrases. Default: `[]`.
/// * `ATA2_PRESENCE_PENALTY`. Default: `0.0`.
/// * `ATA2_FREQUENCY_PENALTY`. Default: `0.0`.
/// * `ATA2_LOGIT_BIAS` sets the logit bias. Default: `{}`.
impl Default for Config {
    fn default() -> Self {
        Self {
            model: env::var("ATA2_MODEL")
                .ok()
                .unwrap_or_else(|| "gpt-3.5-turbo".to_string()),
            max_tokens: env::var("ATA2_MAX_TOKENS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(2048),
            temperature: env::var("ATA2_TEMPERATURE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.8),
            suffix: env::var("ATA2_SUFFIX").ok(),
            top_p: env::var("ATA2_TOP_P")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1.0),
            n: env::var("ATA2_N")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1),
            stream: true,
            stop: env::var("ATA2_STOP")
                .ok()
                .map(|s| serde_json::from_str(&s).unwrap())
                .unwrap_or_else(|| vec![]),
            presence_penalty: env::var("ATA2_PRESENCE_PENALTY")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0),
            frequency_penalty: env::var("ATA2_FREQUENCY_PENALTY")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0),
            logit_bias: env::var("ATA2_LOGIT_BIAS")
                .ok()
                .map(|s| serde_json::from_str(&s).unwrap())
                .unwrap_or_else(|| HashMap::default()),
            api_key: env::var("OPENAI_API_KEY").ok(),
            user_id: env::var("ATA2_USER_ID").ok(),
            ui: UiConfig::default(),
        }
    }
}

/// Note: the result is heavily based on the environment variables.
///
/// * `ATA2_DOUBLE_CTRLC` sets whether to require user to press ^C twice. Default: `true`.
/// * `ATA2_HIDE_CONFIG` sets whether to hide config on run. Default: `false`.
/// * `ATA2_REDACT_API_KEY` sets whether to redact API key. Default: `true`.
/// * `ATA2_MULTILINE_INSERTIONS` sets whether to allow multiline insertions. Default: `true`.
/// * `ATA2_SAVE_HISTORY` sets whether to save history. Default: `true`.
/// * `ATA2_HISTORY_FILE` sets the history file. Default: `~/.config/ata2/history`.
impl Default for UiConfig {
    fn default() -> Self {
        Self {
            double_ctrlc: env::var("ATA2_DOUBLE_CTRLC")
                .ok()
                .map(|s| s.len() > 0)
                .unwrap_or(true),
            hide_config: env::var("ATA2_HIDE_CONFIG")
                .ok()
                .map(|s| s.len() > 0)
                .unwrap_or(false),
            redact_api_key: env::var("ATA2_REDACT_API_KEY")
                .ok()
                .map(|s| s.len() > 0)
                .unwrap_or(true),
            multiline_insertions: env::var("ATA2_MULTILINE_INSERTIONS")
                .ok()
                .map(|s| s.len() > 0)
                .unwrap_or(true),
            save_history: env::var("ATA2_SAVE_HISTORY")
                .ok()
                .map(|s| s.len() > 0)
                .unwrap_or(true),
            history_file: env::var("ATA2_HISTORY_FILE")
                .ok()
                .map(|s| PathBuf::from(s))
                .unwrap_or_else(|| {
                    get_config_dir::<2>()
                        .join("history")
                        .to_string_lossy()
                        .to_string()
                        .into()
                }),
        }
    }
}

impl UiConfig {
    pub fn validate(&self) -> Result<(), String> {
        let history_dir = match self.history_file.parent() {
            Some(dir) => dir,
            None => return Err(String::from("History file has no parent")),
        };

        let history_metadata = match history_dir.metadata() {
            Ok(metadata) => metadata,
            Err(e) => return Err(format!("History file metadata error: {}", e)),
        };

        if history_metadata.permissions().readonly() {
            return Err(String::from("History file dir is read-only"));
        }

        Ok(())
    }
}

impl<'a> Into<OpenAIConfig> for &'a Config {
    fn into(self) -> OpenAIConfig {
        let mut ret = OpenAIConfig::new();
        if let Some(api_key) = &self.api_key {
            ret = ret.with_api_key(api_key.to_owned());
        }
        ret
    }
}

impl<'a> Into<CreateChatCompletionRequestArgs> for &'a Config {
    fn into(self) -> CreateChatCompletionRequestArgs {
        if !self.stream {
            warn!("Stream is disabled. This is not supported anymore and will be ignored.");
        }
        let mut args = CreateChatCompletionRequestArgs::default()
            .n(self.n as u8)
            .model(&self.model)
            .max_tokens(self.max_tokens as u16)
            .temperature(self.temperature as f32)
            .frequency_penalty(self.frequency_penalty as f32)
            .presence_penalty(self.presence_penalty as f32)
            .logit_bias(
                self.logit_bias
                    .clone()
                    .into_iter()
                    .map(|(k, v)| (k, serde_json::Value::Number(Number::from_f64(v).unwrap())))
                    .collect::<StdHashMap<String, Value>>(),
            )
            .top_p(self.top_p as f32)
            .stop(self.stop.clone())
            .stream(true)
            .to_owned();

        if let Some(user_id) = &self.user_id {
            args = args.user(user_id).to_owned();
        }

        args
    }
}

fn fmt_reflectable(f: &mut fmt::Formatter<'_>, value: &dyn Struct) -> Result<(), fmt::Error> {
    write!(f, "{{")?;
    let num_fields = value.iter_fields().count();
    for (i, v) in value.iter_fields().enumerate() {
        let key = value.name_at(i).unwrap();
        if i == num_fields - 1 {
            write!(f, "{}: {:?}", key, v)?;
        } else {
            write!(f, "{}: {:?}, ", key, v)?;
        }
    }
    write!(f, "}}")
}

impl Display for UiConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        fmt_reflectable(f, self)
    }
}

#[derive(Clone, Deserialize, Debug, Default)]
pub enum ConfigLocation {
    #[default]
    Auto,
    Path(PathBuf),
    Named(PathBuf),
}

impl FromStr for ConfigLocation {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(if !s.contains(".") && s.len() > 0 {
            Self::Named(s.into())
        } else if s.trim().len() > 0 {
            Self::Path(s.into())
        } else if s.trim().is_empty() {
            Self::Auto
        } else {
            unreachable!()
        })
    }
}

impl<S> From<S> for ConfigLocation
where
    S: AsRef<str>,
{
    fn from(s: S) -> Self {
        Self::from_str(s.as_ref()).unwrap()
    }
}

fn get_config_dir<const V: usize>() -> PathBuf {
    ProjectDirs::from(
        if V == 1 {
            "ata"
        } else if V == 2 {
            "ata2"
        } else {
            unreachable!()
        },
        "Ask the Terminal Anything (ATA) Project Authors",
        if V == 1 {
            "ata"
        } else if V == 2 {
            "ata2"
        } else {
            unreachable!()
        },
    )
    .unwrap()
    .config_dir()
    .into()
}

pub fn default_path<const V: usize>(name: Option<&Path>) -> PathBuf {
    let mut config_file = get_config_dir::<V>().to_path_buf();
    let file: Vec<_> = if let Some(name) = name {
        let mut name = name.to_path_buf();
        name.set_extension("toml");
        name.as_os_str()
            .to_raw_bytes()
            .into_iter()
            .map(|i| *i)
            .collect()
    } else {
        let name = DEFAULT_CONFIG_FILENAME.to_string_lossy();
        name.bytes().collect()
    };
    let file = OsString::assert_from_raw_vec(file);
    config_file.push(&file);
    config_file
}

impl ConfigLocation {
    pub fn location(&self) -> PathBuf {
        match self {
            ConfigLocation::Auto => {
                let config_dir = get_config_dir::<2>().to_path_buf();
                if DEFAULT_CONFIG_FILENAME.exists() {
                    warn!(
                        "{} found in working directory BUT UNSPECIFIED. \
                          This behavior is DEPRECATED. \
                          Please move it to {}.",
                        DEFAULT_CONFIG_FILENAME.display(),
                        config_dir.display()
                    );
                    return DEFAULT_CONFIG_FILENAME.clone();
                }
                default_path::<2>(None)
            }
            ConfigLocation::Path(pb) => pb.clone(),
            ConfigLocation::Named(name) => default_path::<2>(Some(name)),
        }
    }

    pub fn location_v1(&self) -> PathBuf {
        default_path::<1>(Some(&Path::new("ata.toml")))
    }
}

impl FromStr for Config {
    type Err = TomlError;

    fn from_str(contents: &str) -> Result<Self, Self::Err> {
        toml::from_str(&contents)
    }
}

impl<S> From<S> for Config
where
    S: AsRef<str>,
{
    fn from(s: S) -> Self {
        Self::from_str(s.as_ref()).unwrap_or_else(|e| panic!("Config parsing failure!: {:?}", e))
    }
}

impl Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let mut header = ColouredStr::new("Configuration:");
        header.underline();
        let mut ok = writeln!(f, "{}", header);
        for (i, value) in self.iter_fields().enumerate() {
            if !ok.is_ok() {
                break;
            }
            let key = self.name_at(i).unwrap();
            let mut value2 = match value.downcast_ref::<UiConfig>() {
                Some(ui) => Some(ui.to_string()),
                // Doing this eliminates quotes around strings
                None => match value.downcast_ref::<String>() {
                    Some(s) => match key {
                        "model" => Some(s.to_uppercase()),
                        _ => Some(s.to_string()),
                    },
                    None => None,
                },
            };
            if self.ui.redact_api_key && key == "api_key" {
                let mut redacted = ColouredStr::new("[redacted]");
                redacted.red();
                value2 = Some(redacted.to_string());
            }

            if let Some(v) = value2 {
                ok = writeln!(f, "{key}: {value}", key = key, value = v);
            } else {
                ok = writeln!(f, "{key}: {value:#?}", key = key, value = value);
            }
        }
        ok
    }
}

//! `$XDG_CONFIG_HOME/outlay/config.toml`. The file is optional and every key has a default.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::tui::keys::Keymap;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Seconds to confirm an applied layout before it is reverted; 0 turns the countdown off.
    pub revert_seconds: u64,
    /// Pixels per Alt-direction nudge.
    pub nudge_step: i32,
    /// Where profiles live.
    pub layouts_dir: String,
    pub animations: bool,
    /// The focus letters: left, down, up, right.
    pub directions: String,
    /// Cell height over cell width; detected from the terminal when omitted.
    pub cell_aspect: Option<f64>,
    /// The old look: double borders on the focused display's parent and the stick target.
    pub double_borders: bool,
    /// Shell commands run after a layout is kept, e.g. to redraw the wallpaper.
    pub post_apply: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            revert_seconds: 15,
            nudge_step: 10,
            layouts_dir: "~/.screenlayout".to_owned(),
            animations: true,
            directions: "hjkl".to_owned(),
            cell_aspect: None,
            double_borders: false,
            post_apply: Vec::new(),
        }
    }
}

impl Config {
    /// `$XDG_CONFIG_HOME/outlay/config.toml`
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("outlay").join("config.toml"))
    }

    /// The config file, or the defaults when there is none.
    pub fn load() -> Result<Self> {
        match Self::path() {
            Some(path) if path.exists() => Self::from_file(&path),
            _ => Ok(Self::default()),
        }
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("could not read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let config: Config = toml::from_str(text)?;
        if config.nudge_step <= 0 {
            bail!("nudge_step must be at least 1, not {}", config.nudge_step);
        }
        if let Some(a) = config.cell_aspect
            && !(a.is_finite() && a > 0.0)
        {
            bail!("cell_aspect must be a positive number, not {a}");
        }
        config.keymap()?;
        Ok(config)
    }

    /// The keymap with the configured direction letters.
    pub fn keymap(&self) -> Result<Keymap> {
        Keymap::new(&self.directions).map_err(anyhow::Error::msg)
    }

    /// `layouts_dir` with a leading `~` expanded.
    pub fn layouts_dir(&self) -> PathBuf {
        expand_home(&self.layouts_dir)
    }
}

fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ if path == "~" => dirs::home_dir().unwrap_or_else(|| PathBuf::from(path)),
        _ => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_a_default() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn reads_the_documented_example() {
        let config = Config::parse(
            r#"
            revert_seconds = 20
            nudge_step = 5
            layouts_dir = "~/layouts"
            animations = false
            directions = "hjkl"
            cell_aspect = 2.1
            double_borders = true
            post_apply = ["feh --bg-fill ~/Pictures/wallpapers/current-wallpaper/*"]
            "#,
        )
        .unwrap();
        assert_eq!(config.revert_seconds, 20);
        assert_eq!(config.nudge_step, 5);
        assert!(!config.animations);
        assert_eq!(config.cell_aspect, Some(2.1));
        assert!(config.double_borders);
        assert_eq!(config.post_apply.len(), 1);
        assert!(config.layouts_dir().ends_with("layouts"));
        assert!(!config.layouts_dir().starts_with("~"));
    }

    #[test]
    fn double_borders_are_off_unless_asked_for() {
        assert!(!Config::parse("").unwrap().double_borders);
        assert!(
            Config::parse("double_borders = true")
                .unwrap()
                .double_borders
        );
        let err = Config::parse("double_borders = 1").unwrap_err().to_string();
        assert!(err.contains("double_borders"), "{err}");
    }

    #[test]
    fn rejects_mistakes_with_a_reason() {
        let err = Config::parse("revert_second = 20").unwrap_err().to_string();
        assert!(err.contains("unknown field `revert_second`"), "{err}");
        let err = Config::parse("nudge_step = 0").unwrap_err().to_string();
        assert!(err.contains("nudge_step"), "{err}");
        let err = Config::parse("cell_aspect = -1.0").unwrap_err().to_string();
        assert!(err.contains("cell_aspect"), "{err}");
        let err = Config::parse(r#"directions = "asdf""#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("already bound"), "{err}");
    }
}

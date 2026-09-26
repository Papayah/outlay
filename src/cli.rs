//! Command-line interface.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::tui::app::{Options, WATCH_INTERVAL};
use crate::tui::session::{Settings, default_revert_file};
use crate::tui::theme::Theme;
use crate::xrandr::{Backend, DryRun, FixtureBackend, XrandrCli, check_session};

#[derive(Debug, Parser)]
#[command(
    name = "outlay",
    version,
    about = "Keyboard-driven xrandr layout editor"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Read state from an `xrandr --verbose` capture; never touches X
    #[arg(
        long,
        global = true,
        value_name = "XRANDR-VERBOSE.TXT",
        conflicts_with = "demo"
    )]
    pub from_file: Option<PathBuf>,

    /// Use the built-in four-output fixture; never touches X
    #[arg(long, global = true)]
    pub demo: bool,

    /// Read the live state, but only simulate applies and show their commands
    #[arg(short = 'n', long, global = true)]
    pub dry_run: bool,

    /// Seconds to keep an applied layout before it reverts; 0 turns the countdown off
    /// [default: 15, or revert_seconds from the config]
    #[arg(long, global = true, value_name = "SECONDS")]
    pub revert_timeout: Option<u64>,

    /// Where profiles live [default: ~/.screenlayout, or layouts_dir from the config]
    #[arg(long, global = true, value_name = "DIR")]
    pub layouts_dir: Option<PathBuf>,

    /// Move displays at once instead of letting them glide into place
    #[arg(long, global = true)]
    pub no_anim: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Print the to-scale diagram and the output table, then exit
    Show,
    /// List outputs, then resolutions with their rates
    List,
    /// Apply a profile (<layouts-dir>/<PROFILE>.sh, or a path) with the automatic revert;
    /// with -n, print the command only
    Apply {
        /// A profile name, or a path to a script
        profile: String,
    },
    /// Save the live layout as an arandr-compatible profile; with -n, print it only
    Save {
        /// A profile name, or a path to a script
        profile: String,
        /// Overwrite a different existing file without asking
        #[arg(short, long)]
        force: bool,
    },
    /// Print the keymap and the commands
    Keys,
    /// Print a shell completion script
    Completions { shell: clap_complete::Shell },
}

impl Cli {
    /// The backend the global flags select: a fixture for `--demo` and `--from-file`, else the
    /// real `xrandr` (simulated with `--dry-run`). Refuses live X where xrandr cannot work.
    pub fn backend(&self) -> Result<Box<dyn Backend>> {
        if self.demo {
            return Ok(Box::new(FixtureBackend::demo()));
        }
        if let Some(path) = &self.from_file {
            return Ok(Box::new(FixtureBackend::from_file(path)?));
        }
        check_session(|name| std::env::var(name).ok()).map_err(anyhow::Error::msg)?;
        let live = XrandrCli::new();
        if self.dry_run {
            return Ok(Box::new(DryRun::new(&live)?));
        }
        Ok(Box::new(live))
    }

    /// Editor settings from the config file and the flags.
    pub fn tui_options(&self, config: &Config) -> Result<(Options, Settings)> {
        let source = if self.demo {
            Some("demo".to_owned())
        } else if let Some(p) = &self.from_file {
            Some(p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ))
        } else if self.dry_run {
            Some("dry run".to_owned())
        } else {
            None
        };
        let options = Options {
            keymap: config.keymap()?,
            theme: Theme::from_env(),
            nudge_step: config.nudge_step,
            double_borders: config.double_borders,
            animations: config.animations && !self.no_anim,
            cell_aspect: config.cell_aspect,
            source,
            watch: Some(WATCH_INTERVAL),
        };
        let settings = Settings {
            revert_seconds: self.revert_timeout.unwrap_or(config.revert_seconds),
            revert_file: default_revert_file(),
            hooks: config.post_apply.clone(),
            hook_timeout: Duration::from_secs(config.post_apply_timeout),
            layouts_dir: Some(self.layouts_dir(config)),
        };
        Ok((options, settings))
    }

    /// `--layouts-dir`, else the config's `layouts_dir`.
    pub fn layouts_dir(&self, config: &Config) -> PathBuf {
        self.layouts_dir
            .clone()
            .unwrap_or_else(|| config.layouts_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn global_flags_work_after_the_subcommand() {
        let cli = Cli::try_parse_from(["outlay", "list", "--demo"]).unwrap();
        assert_eq!(cli.command, Some(Command::List));
        assert!(cli.demo);
        assert!(Cli::try_parse_from(["outlay", "--demo", "--from-file", "x.txt"]).is_err());
    }

    #[test]
    fn apply_flags() {
        let cli = Cli::try_parse_from(["outlay", "-n", "--revert-timeout", "0"]).unwrap();
        assert!(cli.dry_run);
        assert_eq!(cli.revert_timeout, Some(0));
        let (options, settings) = cli.tui_options(&Config::default()).unwrap();
        assert_eq!(options.source.as_deref(), Some("dry run"));
        assert_eq!(settings.revert_seconds, 0);

        let cli = Cli::try_parse_from(["outlay", "--demo"]).unwrap();
        let config = Config {
            revert_seconds: 30,
            post_apply: vec!["true".to_owned()],
            post_apply_timeout: 3,
            ..Config::default()
        };
        let (_, settings) = cli.tui_options(&config).unwrap();
        assert_eq!(settings.revert_seconds, 30);
        assert_eq!(settings.hooks, ["true"]);
        assert_eq!(settings.hook_timeout, Duration::from_secs(3));
    }

    #[test]
    fn profile_commands_and_the_layouts_dir() {
        let cli = Cli::try_parse_from(["outlay", "apply", "home", "-n"]).unwrap();
        assert_eq!(
            cli.command,
            Some(Command::Apply {
                profile: "home".to_owned()
            })
        );
        assert!(cli.dry_run);
        let cli =
            Cli::try_parse_from(["outlay", "save", "-f", "x/y.sh", "--layouts-dir", "/l"]).unwrap();
        assert_eq!(
            cli.command,
            Some(Command::Save {
                profile: "x/y.sh".to_owned(),
                force: true
            })
        );
        let config = Config {
            layouts_dir: "/from/config".to_owned(),
            ..Config::default()
        };
        assert_eq!(cli.layouts_dir(&config), PathBuf::from("/l"));
        let cli = Cli::try_parse_from(["outlay"]).unwrap();
        assert_eq!(cli.layouts_dir(&config), PathBuf::from("/from/config"));
        let (_, settings) = cli.tui_options(&config).unwrap();
        assert_eq!(settings.layouts_dir, Some(PathBuf::from("/from/config")));
    }

    #[test]
    fn animations_follow_the_config_and_no_anim() {
        let cli = Cli::try_parse_from(["outlay"]).unwrap();
        assert!(cli.tui_options(&Config::default()).unwrap().0.animations);
        let off = Config {
            animations: false,
            ..Config::default()
        };
        assert!(!cli.tui_options(&off).unwrap().0.animations);
        let cli = Cli::try_parse_from(["outlay", "--no-anim"]).unwrap();
        assert!(!cli.tui_options(&Config::default()).unwrap().0.animations);
    }
}

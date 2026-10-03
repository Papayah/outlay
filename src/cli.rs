//! Command-line interface.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

use crate::backend::{Backend, DryRun, FixtureBackend, compositor_label, detect};
use crate::config::Config;
use crate::tui::app::{Options, WATCH_INTERVAL};
use crate::tui::session::{Settings, default_revert_file};
use crate::tui::theme::Theme;

#[derive(Debug, Parser)]
#[command(
    name = "outlay",
    version,
    about = "Keyboard-driven xrandr layout editor"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Read state from a capture (`xrandr --verbose`, or `wlr-randr --json` and `outlay dump` on
    /// Wayland); never touches the displays
    #[arg(long, global = true, value_name = "CAPTURE", conflicts_with = "demo")]
    pub from_file: Option<PathBuf>,

    /// Use a built-in four-output fixture, X11 unless =wayland; never touches the displays
    #[arg(
        long,
        global = true,
        value_name = "KIND",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "x11"
    )]
    pub demo: Option<Demo>,

    /// Read the live state, but only simulate applies and show their commands
    #[arg(short = 'n', long, global = true)]
    pub dry_run: bool,

    /// Seconds to keep an applied layout before it reverts; 0 turns the countdown off
    /// [default: 15, or revert_seconds from the config]
    #[arg(long, global = true, value_name = "SECONDS")]
    pub revert_timeout: Option<u64>,

    /// Where profiles live on X11 [default: ~/.screenlayout, or layouts_dir from the config]
    #[arg(long, global = true, value_name = "DIR")]
    pub layouts_dir: Option<PathBuf>,

    /// The kanshi config, where profiles live on Wayland [default: $XDG_CONFIG_HOME/kanshi/config,
    /// or kanshi_config from the config]
    #[arg(long, global = true, value_name = "FILE")]
    pub kanshi_config: Option<PathBuf>,

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
    /// Apply a profile (<layouts-dir>/<PROFILE>.sh or a path on X11, a kanshi profile on
    /// Wayland) with the automatic revert; with -n, print the command only
    Apply {
        /// A profile name, or a path to a script
        profile: String,
    },
    /// Save the live layout as a profile (an arandr-compatible script on X11, a block in the
    /// kanshi config on Wayland); with -n, print the file only
    Save {
        /// A profile name, or a path to a script
        profile: String,
        /// Overwrite a different existing file without asking
        #[arg(short, long)]
        force: bool,
    },
    /// Print the keymap and the commands
    Keys,
    /// Print the state as the backend reads it (`xrandr --verbose`, or JSON on Wayland), for bug
    /// reports and --from-file
    Dump,
    /// Print a shell completion script
    Completions { shell: clap_complete::Shell },
    /// Apply the layout a capture describes, at once (what a Wayland revert.sh runs)
    #[command(hide = true)]
    Restore {
        /// A capture file, or - for stdin
        capture: String,
    },
}

/// Which built-in fixture `--demo` uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Demo {
    X11,
    Wayland,
}

impl Cli {
    /// The backend the global flags select: a fixture for `--demo` and `--from-file`, else the
    /// live display server [`detect`] finds (simulated with `--dry-run`).
    pub fn backend(&self) -> Result<Box<dyn Backend>> {
        match self.demo {
            Some(Demo::X11) => return Ok(Box::new(FixtureBackend::demo())),
            Some(Demo::Wayland) => return Ok(Box::new(FixtureBackend::demo_wayland())),
            None => {}
        }
        if let Some(path) = &self.from_file {
            return Ok(Box::new(FixtureBackend::from_file(path)?));
        }
        let live = detect(&|name| std::env::var(name).ok())?;
        if self.dry_run {
            return Ok(Box::new(DryRun::new(live.as_ref())?));
        }
        Ok(live)
    }

    /// Editor settings from the config file and the flags.
    pub fn tui_options(&self, config: &Config) -> Result<(Options, Settings)> {
        let source = if let Some(demo) = self.demo {
            Some(match demo {
                Demo::X11 => "demo".to_owned(),
                Demo::Wayland => "wayland demo".to_owned(),
            })
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
        // Only a live session has a compositor; fixtures stay the same on every machine.
        let live = self.demo.is_none() && self.from_file.is_none();
        let compositor = live
            .then(|| compositor_label(&|name| std::env::var(name).ok()))
            .flatten();
        let options = Options {
            keymap: config.keymap()?,
            theme: Theme::from_env(),
            nudge_step: config.nudge_step,
            double_borders: config.double_borders,
            animations: config.animations && !self.no_anim,
            cell_aspect: config.cell_aspect,
            source,
            compositor,
            watch: Some(WATCH_INTERVAL),
        };
        let settings = Settings {
            revert_seconds: self.revert_timeout.unwrap_or(config.revert_seconds),
            revert_file: default_revert_file(),
            hooks: config.post_apply.clone(),
            hook_timeout: Duration::from_secs(config.post_apply_timeout),
            layouts_dir: Some(self.layouts_dir(config)),
            kanshi_config: self.kanshi_config(config),
            restore_program: Some(restore_program()),
        };
        Ok((options, settings))
    }

    /// `--layouts-dir`, else the config's `layouts_dir`.
    pub fn layouts_dir(&self, config: &Config) -> PathBuf {
        self.layouts_dir
            .clone()
            .unwrap_or_else(|| config.layouts_dir())
    }

    /// `--kanshi-config`, else the config's `kanshi_config`, else the file kanshi reads.
    pub fn kanshi_config(&self, config: &Config) -> Option<PathBuf> {
        self.kanshi_config
            .clone()
            .or_else(|| config.kanshi_config())
    }
}

/// The program a Wayland `revert.sh` runs: this binary, unless it is gone (`install.sh` replaced
/// it while outlay ran, and Linux then names it "… (deleted)"); then `outlay` on `PATH`.
fn restore_program() -> PathBuf {
    std::env::current_exe()
        .ok()
        .filter(|p| p.is_file() && !p.to_string_lossy().ends_with(" (deleted)"))
        .unwrap_or_else(|| PathBuf::from("outlay"))
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
        assert_eq!(cli.demo, Some(Demo::X11));
        assert!(Cli::try_parse_from(["outlay", "--demo", "--from-file", "x.txt"]).is_err());
    }

    #[test]
    fn the_demo_takes_its_kind_after_an_equals_sign() {
        let demo = |args: &[&str]| Cli::try_parse_from(args).map(|c| (c.demo, c.command));
        assert_eq!(
            demo(&["outlay", "--demo=wayland", "show"]).unwrap(),
            (Some(Demo::Wayland), Some(Command::Show))
        );
        assert_eq!(
            demo(&["outlay", "--demo", "list"]).unwrap(),
            (Some(Demo::X11), Some(Command::List)),
            "a word after --demo is the subcommand, not its value"
        );
        assert_eq!(
            demo(&["outlay", "--demo=x11"]).unwrap(),
            (Some(Demo::X11), None)
        );
        assert!(demo(&["outlay", "--demo=gnome"]).is_err());
        let cli = Cli::try_parse_from(["outlay", "--demo=wayland"]).unwrap();
        let (options, _) = cli.tui_options(&Config::default()).unwrap();
        assert_eq!(options.source.as_deref(), Some("wayland demo"));
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
    fn the_kanshi_config_comes_from_the_flag_then_the_config() {
        let config = Config {
            kanshi_config: Some("/from/config".to_owned()),
            ..Config::default()
        };
        let cli = Cli::try_parse_from(["outlay", "--kanshi-config", "/k", "save", "x"]).unwrap();
        assert_eq!(cli.kanshi_config(&config), Some(PathBuf::from("/k")));
        let cli = Cli::try_parse_from(["outlay"]).unwrap();
        let (_, settings) = cli.tui_options(&config).unwrap();
        assert_eq!(settings.kanshi_config, Some(PathBuf::from("/from/config")));
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

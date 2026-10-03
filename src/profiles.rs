//! Where profiles live. On X11 they are screenlayout scripts, one per file in a directory
//! (`~/.screenlayout`); on Wayland they are the profile blocks of kanshi's config, which kanshi
//! applies again whenever displays come and go. The editor (`w`, `e`) and `outlay apply`/`outlay
//! save` all go through a [`ProfileStore`], picked by the kind of display server, so both read
//! and write profiles alike.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::files::write_atomic;
use crate::model::layout::Layout;
use crate::model::profile::Profile;
use crate::model::{Kind, Snapshot};
use crate::wayland::kanshi;
use crate::xrandr::{command, script};

/// What the status line adds after a kanshi profile is saved: outlay never reloads kanshi.
pub const KANSHI_RELOAD: &str = "Run `kanshictl reload` so kanshi uses it.";

/// Where profiles are read from and saved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileStore {
    /// A directory of `#!/bin/sh` scripts: `<dir>/<name>.sh`.
    Screenlayout(PathBuf),
    /// kanshi's config file, which holds every profile; it may include others.
    Kanshi(PathBuf),
}

/// A profile read from a store.
#[derive(Clone, Debug, PartialEq)]
pub struct Stored {
    pub name: String,
    /// The file that holds it.
    pub path: PathBuf,
    pub profile: Profile,
}

/// What saving a profile would write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Save {
    pub name: String,
    pub path: PathBuf,
    /// The file as it is now; `None` when it does not exist yet.
    pub old: Option<String>,
    /// The whole file after the save.
    pub text: String,
}

/// Why a profile could not be read.
#[derive(Debug)]
pub enum ReadError {
    /// The file that should hold it could not be read.
    Io { path: PathBuf, source: io::Error },
    /// The kanshi config has no profile with this name.
    Missing { name: String, path: PathBuf },
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "could not read {}: {source}", path.display()),
            Self::Missing { name, path } => {
                write!(f, "there is no profile {name} in {}", path.display())
            }
        }
    }
}

impl std::error::Error for ReadError {}

impl ReadError {
    /// The file the error is about.
    pub fn path(&self) -> &Path {
        match self {
            Self::Io { path, .. } | Self::Missing { path, .. } => path,
        }
    }

    /// The error as the editor's status line says it, with `shown` for the path.
    pub fn sentence(&self, shown: &str) -> String {
        match self {
            Self::Io { source, .. } => format!("Could not read {shown}: {source}."),
            Self::Missing { name, .. } => format!("There is no profile {name} in {shown}."),
        }
    }
}

/// kanshi's config, or `None` when there is none yet.
fn read_kanshi(path: &Path) -> Result<Option<kanshi::Config>, ReadError> {
    match kanshi::read(path) {
        Ok(config) => Ok(Some(config)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ReadError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

/// The profiles of a kanshi config.
fn kanshi_profiles(config: kanshi::Config) -> Vec<Stored> {
    config
        .profiles
        .into_iter()
        .map(|block| Stored {
            path: config.files[block.file].0.clone(),
            name: block.name,
            profile: block.profile,
        })
        .collect()
}

/// Reads a file that may not exist yet.
fn read_optional(path: &Path) -> Result<Option<String>, ReadError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ReadError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

impl ProfileStore {
    /// The store for this kind of display server, or why there is none: screenlayout scripts
    /// in `layouts_dir` on X11, the kanshi config at `kanshi_config` on Wayland.
    pub fn for_kind(
        kind: Kind,
        layouts_dir: Option<&Path>,
        kanshi_config: Option<&Path>,
    ) -> Result<Self, &'static str> {
        match kind {
            Kind::X11 => layouts_dir
                .map(|dir| Self::Screenlayout(dir.to_owned()))
                .ok_or("No layouts directory is set."),
            Kind::Wayland => kanshi_config
                .map(|path| Self::Kanshi(path.to_owned()))
                .ok_or("No kanshi config is set."),
        }
    }

    /// The directory or file that holds the profiles.
    pub fn location(&self) -> &Path {
        match self {
            Self::Screenlayout(dir) => dir,
            Self::Kanshi(path) => path,
        }
    }

    /// What the status line adds after a save.
    pub fn after_save(&self) -> Option<&'static str> {
        match self {
            Self::Screenlayout(_) => None,
            Self::Kanshi(_) => Some(KANSHI_RELOAD),
        }
    }

    /// Every profile, in the order the picker lists them. A missing directory has none; files
    /// that cannot be read are left out.
    pub fn list(&self) -> io::Result<Vec<Stored>> {
        match self {
            Self::Screenlayout(dir) => {
                let paths = match script::list_profiles(dir) {
                    Ok(paths) => paths,
                    Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
                    Err(err) => return Err(err),
                };
                Ok(paths
                    .into_iter()
                    .filter_map(|path| {
                        let text = std::fs::read_to_string(&path).ok()?;
                        Some(Stored {
                            name: script::profile_name(&path),
                            profile: Profile::parse(&text),
                            path,
                        })
                    })
                    .collect())
            }
            Self::Kanshi(path) => match kanshi::read(path) {
                Ok(config) => Ok(kanshi_profiles(config)),
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
                Err(err) => Err(err),
            },
        }
    }

    /// One profile, by name; a script may also be named by its path.
    pub fn read(&self, name: &str) -> Result<Stored, ReadError> {
        match self {
            Self::Screenlayout(dir) => {
                let path = script::profile_path(dir, name);
                match std::fs::read_to_string(&path) {
                    Ok(text) => Ok(Stored {
                        name: script::profile_name(&path),
                        profile: Profile::parse(&text),
                        path,
                    }),
                    Err(source) => Err(ReadError::Io { path, source }),
                }
            }
            Self::Kanshi(path) => {
                let config = kanshi::read(path).map_err(|source| ReadError::Io {
                    path: path.clone(),
                    source,
                })?;
                kanshi_profiles(config)
                    .into_iter()
                    .find(|s| s.name == name)
                    .ok_or_else(|| ReadError::Missing {
                        name: name.to_owned(),
                        path: path.clone(),
                    })
            }
        }
    }

    /// What saving `layout`, on `snap`, as profile `name` would write. An existing file keeps
    /// everything but the profile itself.
    pub fn save(&self, name: &str, layout: &Layout, snap: &Snapshot) -> Result<Save, ReadError> {
        match self {
            Self::Screenlayout(dir) => {
                let path = script::profile_path(dir, name);
                let old = read_optional(&path)?;
                let text = script::save_text(old.as_deref(), &command::script_command(layout));
                Ok(Save {
                    name: script::profile_name(&path),
                    path,
                    old,
                    text,
                })
            }
            Self::Kanshi(main) => {
                let config = read_kanshi(main)?;
                let (path, text) = kanshi::save(config.as_ref(), main, name, layout, snap);
                let old = match config
                    .iter()
                    .flat_map(|c| &c.files)
                    .find(|(p, _)| *p == path)
                {
                    Some((_, text)) => Some(text.clone()),
                    None => read_optional(&path)?,
                };
                Ok(Save {
                    name: name.to_owned(),
                    path,
                    old,
                    text,
                })
            }
        }
    }

    /// Writes a save, atomically. A new script is executable.
    pub fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        match self {
            Self::Screenlayout(_) => write_atomic(path, text, 0o755),
            Self::Kanshi(_) => write_atomic(path, text, 0o644),
        }
    }
}

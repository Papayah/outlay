//! Where profiles live. On X11 they are screenlayout scripts, one per file in a directory
//! (`~/.screenlayout`). The editor (`w`, `e`) and `outlay apply`/`outlay save` all go through a
//! [`ProfileStore`], picked by the kind of display server, so both read and write profiles alike.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::files::write_atomic;
use crate::model::Kind;
use crate::model::layout::Layout;
use crate::model::profile::Profile;
use crate::xrandr::{command, script};

/// Why `w`, `e`, `outlay apply` and `outlay save` do nothing on Wayland yet.
pub const NO_WAYLAND_PROFILES: &str =
    "Profiles on Wayland are kanshi profiles, which arrive in the next version of outlay.";

/// Where profiles are read from and saved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileStore {
    /// A directory of `#!/bin/sh` scripts: `<dir>/<name>.sh`.
    Screenlayout(PathBuf),
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
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "could not read {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for ReadError {}

impl ReadError {
    /// The file the error is about.
    pub fn path(&self) -> &Path {
        match self {
            Self::Io { path, .. } => path,
        }
    }

    /// The error as the editor's status line says it, with `shown` for the path.
    pub fn sentence(&self, shown: &str) -> String {
        match self {
            Self::Io { source, .. } => format!("Could not read {shown}: {source}."),
        }
    }
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
    /// The store for this kind of display server, or why there is none. `layouts_dir` is where
    /// screenlayout scripts live.
    pub fn for_kind(kind: Kind, layouts_dir: Option<&Path>) -> Result<Self, &'static str> {
        match kind {
            Kind::X11 => layouts_dir
                .map(|dir| Self::Screenlayout(dir.to_owned()))
                .ok_or("No layouts directory is set."),
            Kind::Wayland => Err(NO_WAYLAND_PROFILES),
        }
    }

    /// The directory or file that holds the profiles.
    pub fn location(&self) -> &Path {
        match self {
            Self::Screenlayout(dir) => dir,
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
        }
    }

    /// What saving `layout` as profile `name` would write. An existing file keeps everything but
    /// the profile itself.
    pub fn save(&self, name: &str, layout: &Layout) -> Result<Save, ReadError> {
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
        }
    }

    /// Writes a save, atomically. A new script is executable.
    pub fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        match self {
            Self::Screenlayout(_) => write_atomic(path, text, 0o755),
        }
    }
}

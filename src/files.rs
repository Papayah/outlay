//! Text files outlay writes: profiles and `revert.sh`. Every write is atomic, and a change to an
//! existing file is shown as a line diff before it is made.
//!
//! An atomic write goes through a symlinked destination: the link stays a link, and its target
//! gets the new text. The text goes to a fresh, hidden temporary file next to that target, created
//! exclusively (so nothing already at that name, a symlink included, is followed), and is then
//! renamed into place. A failed write removes the temporary file and leaves the old file as it was.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// How many symlinks `write_atomic` follows before it gives up, as the kernel's `MAXSYMLINKS`.
const MAX_SYMLINK_HOPS: usize = 40;

/// Numbers the temporary files of one process, so concurrent writes never share one.
static TMP_COUNTER: AtomicU32 = AtomicU32::new(0);

/// A line diff from `old` to `new`: kept lines start with two spaces, removed ones with `- `,
/// added ones with `+ `.
pub fn line_diff(old: &str, new: &str) -> Vec<String> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    // Longest common subsequence, filled from the end.
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.push(format!("  {}", a[i]));
            i += 1;
            j += 1;
        } else if i < a.len() && (j == b.len() || lcs[i + 1][j] >= lcs[i][j + 1]) {
            out.push(format!("- {}", a[i]));
            i += 1;
        } else {
            out.push(format!("+ {}", b[j]));
            j += 1;
        }
    }
    out
}

/// Writes a file atomically and syncs it to disk: a fresh temporary file next to it, then a
/// rename. A new file gets `mode`; an existing one keeps its permissions. A symlinked `path` is
/// written through: the link stays, and its final target gets the text. A dangling link has its
/// target created, but not the directories on the way to it.
pub fn write_atomic(path: &Path, text: &str, mode: u32) -> io::Result<()> {
    write(path, text, mode, true)
}

/// [`write_atomic`] without the syncs, for `revert.sh`: it only has to outlive outlay, not a power
/// cut, and an apply should not wait for the disk.
pub fn write_atomic_unsynced(path: &Path, text: &str, mode: u32) -> io::Result<()> {
    write(path, text, mode, false)
}

fn write(path: &Path, text: &str, mode: u32, sync: bool) -> io::Result<()> {
    let target = resolve_symlinks(path)?;
    if target != path {
        // No directory is made at the far end of a link: a link into a deleted dotfiles repo or
        // an unmounted disk fails instead of growing a tree there.
        return replace(&target, text, mode, sync).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("it is a symlink to {}: {e}", target.display()),
            )
        });
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    replace(path, text, mode, sync)
}

/// Replaces `target`, which is not a symlink, with `text`: a temporary file, then a rename.
fn replace(target: &Path, text: &str, mode: u32, sync: bool) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = match std::fs::metadata(target) {
        Ok(meta) => meta.permissions(),
        Err(_) => std::fs::Permissions::from_mode(mode),
    };
    remove_stale_temporaries(target);
    let (tmp, mut file) = create_temporary(target, &TMP_COUNTER)?;
    let written = file
        .write_all(text.as_bytes())
        // On the handle (`fchmod`), so the umask does not apply.
        .and_then(|()| file.set_permissions(permissions))
        .and_then(|()| if sync { file.sync_all() } else { Ok(()) })
        .and_then(|()| std::fs::rename(&tmp, target));
    match written {
        Ok(()) if sync => {
            // The rename reaches the disk with the directory. A directory that cannot be synced
            // (some filesystems refuse) does not undo the write, so that error is not reported.
            if let Ok(dir) = File::open(directory_of(target)) {
                let _ = dir.sync_all();
            }
        }
        Ok(()) => {}
        Err(_) => {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    written
}

/// The directory a file is in, `.` for a bare name.
fn directory_of(path: &Path) -> &Path {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    }
}

/// The start of the temporary file names for `target`: `.<name>.outlay-tmp-`, which
/// `<pid>-<n>` completes. The leading dot keeps them out of globs such as kanshi's
/// `include config.d/*`.
fn temporary_prefix(target: &Path) -> io::Result<OsString> {
    let name = target.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} does not name a file", target.display()),
        )
    })?;
    let mut prefix = OsString::from(".");
    prefix.push(name);
    prefix.push(".outlay-tmp-");
    Ok(prefix)
}

/// The `n`th temporary file name of this process for `target`.
fn temporary_name(target: &Path, prefix: &OsStr, n: u32) -> PathBuf {
    let mut name = prefix.to_owned();
    name.push(format!("{}-{n}", std::process::id()));
    target.with_file_name(name)
}

/// Creates a temporary file for `target`, exclusively (`O_EXCL`), so nothing already at the name
/// is followed or reused; a taken name moves on to the next number from `counter`.
fn create_temporary(target: &Path, counter: &AtomicU32) -> io::Result<(PathBuf, File)> {
    use std::os::unix::fs::OpenOptionsExt;
    let prefix = temporary_prefix(target)?;
    loop {
        let tmp = temporary_name(target, &prefix, counter.fetch_add(1, Ordering::Relaxed));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
        {
            Ok(file) => return Ok((tmp, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

/// Removes the temporary files an outlay left next to `target` when it was killed between making
/// one and renaming it: those whose process is gone. Anything else is left alone.
fn remove_stale_temporaries(target: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let Ok(prefix) = temporary_prefix(target) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(directory_of(target)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid = name
            .as_bytes()
            .strip_prefix(prefix.as_bytes())
            .and_then(|rest| std::str::from_utf8(rest).ok())
            .and_then(|rest| rest.split_once('-'))
            .and_then(|(pid, n)| n.parse::<u32>().ok().and(pid.parse().ok()))
            .and_then(rustix::process::Pid::from_raw);
        if let Some(pid) = pid
            && rustix::process::test_kill_process(pid) == Err(rustix::io::Errno::SRCH)
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Follows `path` while it is a symlink and returns the path it finally names, which need not
/// exist. A relative link target is taken from the link's directory. Like the kernel's
/// `protected_symlinks`, a link that someone else planted is refused (see [`may_follow`]).
fn resolve_symlinks(path: &Path) -> io::Result<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let user = rustix::process::geteuid().as_raw();
    let mut current = path.to_path_buf();
    for _ in 0..MAX_SYMLINK_HOPS {
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let dir = directory_of(&current);
                let dir_owner = std::fs::metadata(dir)?.uid();
                if !may_follow(meta.uid(), dir_owner, user) {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!(
                            "{} is a symlink of another user (uid {}), so it is not followed",
                            current.display(),
                            meta.uid()
                        ),
                    ));
                }
                let link = std::fs::read_link(&current)?;
                current = match current.parent() {
                    Some(dir) if link.is_relative() => dir.join(link),
                    _ => link,
                };
            }
            _ => return Ok(current),
        }
    }
    Err(io::Error::other(format!(
        "{}: more than {MAX_SYMLINK_HOPS} symlinks",
        path.display()
    )))
}

/// Whether a symlink may be followed: it belongs to the user, to the owner of its directory, or
/// to root. Anyone else could only have planted it.
fn may_follow(link_owner: u32, dir_owner: u32, user: u32) -> bool {
    link_owner == user || link_owner == dir_owner || link_owner == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_diffs() {
        assert_eq!(
            line_diff("a\nb\nc\n", "a\nx\nc\nd\n"),
            ["  a", "- b", "+ x", "  c", "+ d"]
        );
        assert_eq!(line_diff("", "a"), ["+ a"]);
    }

    /// A fresh directory for one test, removed when the guard drops.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let dir = std::env::temp_dir().join(format!("outlay-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn mode(p: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    fn set_mode(p: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    fn is_symlink(p: &Path) -> bool {
        std::fs::symlink_metadata(p)
            .unwrap()
            .file_type()
            .is_symlink()
    }

    /// The temporary files left in `dir`.
    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".outlay-tmp"))
            .collect()
    }

    #[test]
    fn atomic_writes_keep_permissions() {
        let dir = TempDir::new("atomic");
        let path = dir.0.join("nested").join("p.sh");
        write_atomic(&path, "one\n", 0o755).unwrap();
        assert_eq!(mode(&path), 0o755);
        set_mode(&path, 0o700);
        write_atomic(&path, "two\n", 0o755).unwrap();
        assert_eq!(mode(&path), 0o700);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two\n");
        assert!(leftovers(path.parent().unwrap()).is_empty());
    }

    #[test]
    fn new_files_get_the_exact_mode() {
        // The mode is set on the handle, so a umask that would clear bits does not apply.
        let dir = TempDir::new("atomic-mode");
        for (name, wanted) in [("revert.sh", 0o755), ("config", 0o644), ("shared", 0o666)] {
            let path = dir.0.join(name);
            write_atomic(&path, "x\n", wanted).unwrap();
            assert_eq!(mode(&path), wanted, "{name}");
        }
    }

    /// The lab's characterization of the old writer, the other way round: a symlink planted at
    /// the old temporary name `<dest>.outlay-tmp` no longer redirects the text or the mode, and the
    /// destination is a regular file.
    #[test]
    fn a_planted_temporary_symlink_redirects_nothing() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new("atomic-planted");
        let victim = dir.0.join("unrelated.txt");
        let destination = dir.0.join("profile.sh");
        std::fs::write(&victim, "original contents").unwrap();
        set_mode(&victim, 0o644);
        symlink(&victim, dir.0.join("profile.sh.outlay-tmp")).unwrap();
        write_atomic(&destination, "new profile\n", 0o700).unwrap();
        assert_eq!(
            std::fs::read_to_string(&victim).unwrap(),
            "original contents"
        );
        assert!(!is_symlink(&destination));
        assert_eq!(mode(&victim), 0o644);
        assert_eq!(mode(&destination), 0o700);
    }

    #[test]
    fn a_symlinked_destination_is_written_through() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new("atomic-link");
        std::fs::create_dir(dir.0.join("dotfiles")).unwrap();
        let target = dir.0.join("dotfiles").join("config");
        std::fs::write(&target, "old\n").unwrap();
        set_mode(&target, 0o600);
        let link = dir.0.join("config");
        // A relative link, then a link to that link.
        symlink("dotfiles/config", &link).unwrap();
        let outer = dir.0.join("outer");
        symlink(&link, &outer).unwrap();

        write_atomic(&outer, "new\n", 0o644).unwrap();
        assert!(is_symlink(&outer));
        assert!(is_symlink(&link));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new\n");
        assert_eq!(mode(&target), 0o600);
        assert!(leftovers(&dir.0).is_empty());
        assert!(leftovers(&dir.0.join("dotfiles")).is_empty());
    }

    #[test]
    fn a_dangling_symlink_gets_its_target_created() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new("atomic-dangling");
        std::fs::create_dir(dir.0.join("elsewhere")).unwrap();
        let link = dir.0.join("p.sh");
        let target = dir.0.join("elsewhere").join("p.sh");
        symlink(&target, &link).unwrap();
        write_atomic(&link, "made\n", 0o755).unwrap();
        assert!(is_symlink(&link));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "made\n");
        assert_eq!(mode(&target), 0o755);
    }

    #[test]
    fn a_dangling_symlink_into_a_missing_directory_makes_nothing() {
        // A link into a deleted dotfiles repo: no directory is made at its far end.
        use std::os::unix::fs::symlink;
        let dir = TempDir::new("atomic-dangling-dir");
        let link = dir.0.join("p.sh");
        let missing = dir.0.join("old-dotfiles");
        symlink(missing.join("screenlayout").join("p.sh"), &link).unwrap();
        let err = write_atomic(&link, "x\n", 0o755).unwrap_err();
        assert!(!missing.exists());
        assert!(is_symlink(&link));
        assert!(err.to_string().contains("old-dotfiles"), "{err}");
    }

    #[test]
    fn an_error_through_a_symlink_names_its_target() {
        // As a link into a read-only /nix/store would: the far end cannot be replaced.
        use std::os::unix::fs::symlink;
        let dir = TempDir::new("atomic-link-error");
        let target = dir.0.join("store").join("config");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("inside"), "kept").unwrap();
        let link = dir.0.join("config");
        symlink(&target, &link).unwrap();
        let err = write_atomic(&link, "x\n", 0o644).unwrap_err();
        let wanted = format!("it is a symlink to {}: ", target.display());
        assert!(err.to_string().starts_with(&wanted), "{err}");
        assert!(leftovers(&dir.0.join("store")).is_empty());
    }

    #[test]
    fn symlinks_of_another_user_are_not_followed() {
        let (user, other, root) = (1000, 1001, 0);
        assert!(may_follow(user, user, user));
        // A link the owner of a shared directory made, or root (`sudo stow`), is trusted.
        assert!(may_follow(other, other, user));
        assert!(may_follow(root, user, user));
        // `sudo outlay` in the user's home.
        assert!(may_follow(user, user, root));
        // Planted: in the user's directory, or in a shared one such as /tmp.
        assert!(!may_follow(other, user, user));
        assert!(!may_follow(other, root, user));
    }

    #[test]
    fn a_symlink_loop_is_an_error() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new("atomic-loop");
        symlink("b", dir.0.join("a")).unwrap();
        symlink("a", dir.0.join("b")).unwrap();
        assert!(write_atomic(&dir.0.join("a"), "x\n", 0o644).is_err());
        assert!(leftovers(&dir.0).is_empty());
    }

    #[test]
    fn a_taken_temporary_name_is_skipped_and_not_followed() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new("atomic-taken");
        let victim = dir.0.join("victim.txt");
        std::fs::write(&victim, "original").unwrap();
        set_mode(&victim, 0o600);
        let target = dir.0.join("p.sh");
        let prefix = temporary_prefix(&target).unwrap();
        let name = |n| temporary_name(&target, &prefix, n);
        // The next names this process would use: a symlink to a victim, a dangling symlink, and
        // a file someone else is writing.
        symlink(&victim, name(0)).unwrap();
        symlink(dir.0.join("missing"), name(1)).unwrap();
        std::fs::write(name(2), "theirs").unwrap();

        let counter = AtomicU32::new(0);
        let (tmp, _file) = create_temporary(&target, &counter).unwrap();
        assert_eq!(tmp, name(3));
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "original");
        assert_eq!(mode(&victim), 0o600);
        assert!(!dir.0.join("missing").exists());
        assert_eq!(std::fs::read_to_string(name(2)).unwrap(), "theirs");
    }

    #[test]
    fn temporary_files_are_hidden() {
        let target = Path::new("/home/u/.config/kanshi/config.d/desk");
        let prefix = temporary_prefix(target).unwrap();
        let name = temporary_name(target, &prefix, 7);
        let wanted = format!(".desk.outlay-tmp-{}-7", std::process::id());
        assert_eq!(name, target.with_file_name(wanted));
    }

    #[test]
    fn stale_temporary_files_are_removed() {
        let dir = TempDir::new("atomic-stale");
        let path = dir.0.join("p.sh");
        // A process that has exited, as an outlay killed before its rename.
        let mut child = std::process::Command::new("true").spawn().unwrap();
        child.wait().unwrap();
        let dead = child.id();
        let stale = dir.0.join(format!(".p.sh.outlay-tmp-{dead}-0"));
        let live = dir
            .0
            .join(format!(".p.sh.outlay-tmp-{}-99", std::process::id()));
        let init = dir.0.join(".p.sh.outlay-tmp-1-0");
        let other = dir.0.join(format!(".q.sh.outlay-tmp-{dead}-0"));
        let odd = dir.0.join(format!(".p.sh.outlay-tmp-{dead}"));
        for file in [&stale, &live, &init, &other, &odd] {
            std::fs::write(file, "half").unwrap();
        }
        write_atomic(&path, "x\n", 0o755).unwrap();
        assert!(!stale.exists());
        for kept in [&live, &init, &other, &odd] {
            assert!(kept.exists(), "{}", kept.display());
        }
    }

    #[test]
    fn a_failed_rename_leaves_no_temporary_file() {
        let dir = TempDir::new("atomic-fail");
        let path = dir.0.join("p.sh");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("inside"), "kept").unwrap();
        assert!(write_atomic(&path, "x\n", 0o755).is_err());
        assert!(leftovers(&dir.0).is_empty());
        assert_eq!(
            std::fs::read_to_string(path.join("inside")).unwrap(),
            "kept"
        );
    }

    #[test]
    fn concurrent_writes_never_share_a_temporary_file() {
        let dir = TempDir::new("atomic-threads");
        let path = dir.0.join("p.sh");
        let texts: Vec<String> = (0..8)
            .map(|i| format!("writer {i}\n").repeat(500))
            .collect();
        std::thread::scope(|s| {
            for (i, text) in texts.iter().enumerate() {
                let path = &path;
                let write = if i % 2 == 0 {
                    write_atomic
                } else {
                    write_atomic_unsynced
                };
                s.spawn(move || write(path, text, 0o644).unwrap());
            }
        });
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(texts.contains(&content));
        assert!(leftovers(&dir.0).is_empty());
    }
}

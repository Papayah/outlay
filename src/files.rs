//! Text files outlay writes: profiles and `revert.sh`. Every write is atomic, and a change to an
//! existing file is shown as a line diff before it is made.
//!
//! An atomic write goes through a symlinked destination: the link stays a link, and its target
//! gets the new text. The text goes to a fresh temporary file next to that target, created
//! exclusively (so nothing already at that name, a symlink included, is followed), and is then
//! renamed into place. A failed write removes the temporary file and leaves the old file as it was.

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

/// Writes a file atomically: a fresh temporary file next to it, then a rename. A new file gets
/// `mode`; an existing one keeps its permissions. A symlinked `path` is written through: the link
/// stays, and its final target gets the text (a dangling link has its target created).
pub fn write_atomic(path: &Path, text: &str, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let target = resolve_symlinks(path)?;
    if let Some(dir) = target.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let permissions = match std::fs::metadata(&target) {
        Ok(meta) => meta.permissions(),
        Err(_) => std::fs::Permissions::from_mode(mode),
    };
    let (tmp, mut file) = loop {
        let mut name = target.as_os_str().to_owned();
        let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        name.push(format!(".outlay-tmp-{}-{n}", std::process::id()));
        let tmp = PathBuf::from(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
        {
            Ok(file) => break (tmp, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let written = file
        .write_all(text.as_bytes())
        // On the handle (`fchmod`), so the umask does not apply.
        .and_then(|()| file.set_permissions(permissions))
        .and_then(|()| file.sync_all())
        .and_then(|()| std::fs::rename(&tmp, &target));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Follows `path` while it is a symlink and returns the path it finally names, which need not
/// exist. A relative link target is taken from the link's directory.
fn resolve_symlinks(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..MAX_SYMLINK_HOPS {
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
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

    #[test]
    fn a_symlink_at_the_old_temporary_name_is_not_followed() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new("atomic-old-tmp");
        let victim = dir.0.join("victim.txt");
        std::fs::write(&victim, "original").unwrap();
        set_mode(&victim, 0o600);
        let path = dir.0.join("p.sh");
        symlink(&victim, dir.0.join("p.sh.outlay-tmp")).unwrap();
        write_atomic(&path, "new\n", 0o755).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "original");
        assert_eq!(mode(&victim), 0o600);
        assert!(!is_symlink(&path));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
    }

    /// The lab's characterization of the old writer, the other way round: a planted temporary
    /// symlink no longer redirects the text or the mode, and the destination is a regular file.
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
        let link = dir.0.join("p.sh");
        let target = dir.0.join("elsewhere").join("p.sh");
        symlink(&target, &link).unwrap();
        write_atomic(&link, "made\n", 0o755).unwrap();
        assert!(is_symlink(&link));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "made\n");
        assert_eq!(mode(&target), 0o755);
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
            for text in &texts {
                let path = &path;
                s.spawn(move || write_atomic(path, text, 0o644).unwrap());
            }
        });
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(texts.contains(&content));
        assert!(leftovers(&dir.0).is_empty());
    }
}

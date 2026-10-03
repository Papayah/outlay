//! Text files outlay writes: profiles and `revert.sh`. Every write is atomic, and a change to an
//! existing file is shown as a line diff before it is made.

use std::io;
use std::path::{Path, PathBuf};

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

/// Writes a file atomically: a temporary file next to it, then a rename. A new file gets
/// `mode`; an existing one keeps its permissions.
pub fn write_atomic(path: &Path, text: &str, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let permissions = match std::fs::metadata(path) {
        Ok(meta) => meta.permissions(),
        Err(_) => std::fs::Permissions::from_mode(mode),
    };
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".outlay-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    std::fs::set_permissions(&tmp, permissions)?;
    std::fs::rename(&tmp, path)
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

    #[test]
    fn atomic_writes_keep_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("outlay-atomic-{}", std::process::id()));
        let path = dir.join("nested").join("p.sh");
        write_atomic(&path, "one\n", 0o755).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o755);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        write_atomic(&path, "two\n", 0o755).unwrap();
        assert_eq!(mode(&path), 0o700);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two\n");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

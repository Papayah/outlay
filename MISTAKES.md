# Traps that cost time

Each entry: symptom → cause → the fix that worked. Append new ones; keep entries that are still true.

## Commits carry the work email

- **Symptom:** a commit shows the global (work) email as author or committer, and GitHub
  attributes it to the wrong account.
- **Cause:** the global git identity on this machine is the work one. A fresh clone or a new
  `git init` inherits it until a local identity is set.
- **Fix:** before the first commit in any clone, run
  `git config user.name "Papayah"` and `git config user.email "maciej.chmiest@gmail.com"`
  (local, never `--global`). After every commit, `git log -1 --format='%an <%ae> | %cn <%ce>'`
  must show Papayah and the gmail address twice. To repair the last commit:
  `git commit --amend --no-edit --reset-author`.

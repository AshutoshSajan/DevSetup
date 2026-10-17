# DevSetup conventions — the rule book

This file is the canonical, human-readable statement of how work happens in
this repository. `AGENTS.md` carries the short, inline hard-rule list that
agent harnesses auto-load (`CLAUDE.md` imports it); when the two ever drift,
**this file wins** and the other must be fixed.

## Who loads what

| File | Role |
|---|---|
| `CONVENTIONS.md` | this file — canonical rules, full depth |
| `AGENTS.md` | machine-facing short rulebook (hard rules inline, self-sufficient) |
| `CLAUDE.md` | two lines: preamble + `@AGENTS.md` import (never duplicate rule text) |
| `ROADMAP.md` | roadmap, todos, working notes (linked from README and here) |
| `README.md` | product facts only: what the app is, how to run/build/deploy |

## Branch model

Only **two trunks** exist. Nothing else — no staging, no sandbox, no alternate
integration tiers.

| trunk | purpose | advances only by |
|---|---|---|
| `dev` | integration trunk — everything lands here first | merge commits from PRs |
| `main` | release trunk — what shipped | release PRs `dev` → `main` |

- Task branches are cut **from `dev`**: `feat/… fix/… chore/… ci/… docs/… perf/… refactor/… test/… release/… dependabot/…`.
- Pull requests target **`dev`** (`gh pr create --base dev`), except release PRs `dev` → `main`.
- Merges into trunks are **merge commits** — never squash, never fast-forward, never rebase-merge. History on trunks stays legible and append-only.
- Dependency updates (Dependabot) also target `dev`.

## Never-list (irreversible actions are not options)

Every rule below is enforceable still after a mistake — the branch of "we can
undo it later" is closed. `1`, `2`, `4` are guarded in local hooks and/or CI
(see "Enforcement").

1. **Trunks are never worked on directly.** No local commits on `dev`/`main`, no
   pushes to them (same branch or otherwise), no merges performed locally into
   them, no `--force` pushes, no resets, no branch deletions on the remote.
   Everything enters a trunk through an approved PR merge.
2. **No database deletion or destruction.** Never drop `*.db` / store files /
   tables, never `TRUNCATE` or bulk-`DELETE`, never destructive or reversed
   migrations — schema changes are additive only. This covers the app's own
   persisted state as well (for example `~/.local/share/devsetup/state.json`
   is user state, not a cache — never "clean it up" from a dev session).
3. **No history rewrites on shared branches.** No rebasing, amending or
   reordering commits that were already pushed to a branch other people or
   agents may have based work on. Rewrite only your own unpushed tips.
4. **No secrets in the repository.** Env files (`*.env`, `.env.local`, keys,
   tokens) stay local. Only `*.example`-style templates may be committed,
   and only with placeholder values. The DevSetup app itself never needs
   repo-held credentials; if you find one, remove it, rotate it and say so.
5. **Generated files are regenerated, never hand-edited.**
   `dist/` (`vite build`), `src-tauri/target/` (`cargo`), icons
   (`npm run icons`), lockfiles (`package-lock.json` / `Cargo.lock`, via
   `npm install|ci` / `cargo` only), install-script pins in
   `manifest/tools.yaml` (`npm run pin`). Hand-edits are lost on the next
   regen anyway — if a generated output is wrong, fix the generator.
6. **Hooks are bypassed only as a deliberate, stated exception.**
   `--no-verify` or disabling `core.hooksPath` is legitimate exactly twice in
   a session: when the hook is *broken at its own job* (say so, fix it), or
   when its exactly-stated exception applies (say what it is — in the PR body).

## Working in a shared checkout

If another session may be working in the same clone:

```bash
git worktree add ../devsetup-<topic> -b <task-branch> origin/dev
# work inside the worktree; never stage or commit a sibling session's files
```

Files that carry uncommitted changes are somebody's in-progress work — leave
them exactly as found.

## Verify before push (the gate)

```bash
git config core.hooksPath .githooks   # one-time after clone
npm run check                          # tsc --noEmit && vite build && cargo test
```

- `.githooks/pre-push` refuses any push to a trunk (commits, `--force`, and
  deletions, detected from the push's ref lines *and* command line, with an
  exact-token boundary regex so `refs/heads/mainfoo` or `refs/heads/dev/x`
  passes) and otherwise runs the gate.
- `.githooks/commit-msg` enforces Conventional Commits
  (`type(scope)?: subject`; merge commits and reverts are exempt).
- `.githooks/post-merge` runs `git fetch --prune origin` (errors ignored — an
  offline merge must stay a working offline merge) so deleted remote branches
  stop ghosting locally.
- The hook clears the git plumbing environment (`GIT_DIR`,
  `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_OBJECT_DIRECTORY`,
  `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_PREFIX`) before running the gate,
  because build tools that shell out to `git` (version stamps and co.) would
  otherwise describe the wrong repository from inside a hook or worktree.

## Enforcement beyond local hooks (CI)

All on the free plan, all payload-only or shallow-cheap:

- `guard-dev.yml` / `guard-main.yml`: on any push to a trunk, jq-read the
  event payload (no checkout). Fail if the push deleted a branch
  (`head_commit` missing), if `forced` is true, or if the head commit is not
  a merge commit (`parents < 2` — squash counts). When the payload itself
  ships no parent list (observed for merges made in the GitHub UI), the
  parent count comes from the REST commits API instead.
- `prune-branch.yml`: after a PR into `dev`/`main` is merged, delete its head
  branch from origin — only if the branch still exists, is on the allowlisted
  namespaces, *is not* a trunk, and the compare API proves the landing
  (`behind` or `identical` means the head commit is contained in the trunk).
  Unproven ⇒ untouched. Payload-only: no checkout. This is the single writer
  for the "delete merged branches" concern: no inline prune step may exist
  elsewhere.
- `deps-audit.yml`: weekly cron (off the round minute) and PRs into `main`
  touching dependency files. Runs a sha-verified `osv-scanner` over
  `package-lock.json` + `Cargo.lock`, `npm audit --omit=dev`, and the stale
  pin check. Findings fail; patch by regenerating locks (allowed commands
  only) or by accepting the Dependabot PRs.

## Dependencies

- `.github/dependabot.yml`: npm (`/`), cargo (`/src-tauri`), `github-actions`
  — all `weekly`, all targeting `dev`.
- Lockfiles change only through package-manager runs; bumps flow in as
  Dependabot PRs into `dev`.
- Local parity: `scripts/deps.sh` (or `--offline` to skip network steps).

## Reviews and PRs

- A PR contains one concern. Convention/tooling PRs (`chore/…`) never ride
  alongside feature work.
- The PR body states what changed and why, plus — when applicable — any
  deliberate hook bypass or pinned-hash refresh.
- Merging your own PR is allowed for a personal repo like this one; the
  guards above replace only part of review, not all of it.

## Roadmap

Roadmap, todos and working notes live in [`ROADMAP.md`](ROADMAP.md) — kept
separate from this rule book so rules stay stable while plans churn.

# AGENTS.md — DevSetup hard rules (auto-loaded)

Short, self-sufficient rulebook for AI agents. Depth and rationale live in
[CONVENTIONS.md](CONVENTIONS.md) — reading it adds depth, it is never required
for compliance.

## Trunks and pull requests

- Only two trunks exist: `dev` (integration) and `main` (release).
- **Never commit or push to the trunks (`dev`, `main`)** — no direct commits,
  no local merges into them, no `--force`, no resets, no branch deletions.
  All of it is refused locally by `.githooks/pre-push` and by CI guards.
- Cut task branches (`feat/… fix/… chore/… ci/… docs/… perf/… refactor/… test/…`) **from `dev`** and
  open PRs only into the integration trunk: `gh pr create --base dev`
  (release PRs `dev` → `main` are the only exception).
- Trunk merges are **merge commits only** — never squash, never fast-forward.
- **Never rewrite shared history** (no rebase/amend of commits already pushed
  to a branch others may build on).
- **Actions that cannot be undone are not options.** If you find yourself
  planning one, stop and present the reversible alternative.
- **Shared checkout:** work in a linked worktree
  (`git worktree add ../devsetup-<topic> -b <branch> origin/dev`) and leave
  another session's uncommitted files untouched.

## Destructive operations

- **Never drop, delete or truncate databases** (`*.db` files, stores, tables)
  and never bulk-`DELETE` rows. No destructive or reversed migrations —
  additive schema changes only. This includes the app's own persisted state.
- **Never commit secrets.** Env files stay local; commit only `*.example`
  templates with placeholders. If a secret lands, remove it, rotate it, and
  state it in the PR.
- **Regenerate; never hand-edit** generated outputs: `dist/`, `src-tauri/target/`,
  icons (`npm run icons`), lockfiles (`npm install` / `cargo` runs only),
  install-script pins (`npm run pin`).
- Bypass hooks only as a deliberate, stated exception (recorded in the PR body).

## Verify before push

- **Run the gate before every push:** `npm run check`
  (= `tsc --noEmit && vite build && cargo test`).
- Enable hooks once after cloning: `git config core.hooksPath .githooks`.
  The hooks then enforce trunk refusals, Conventional Commits subjects and the
  gate automatically.
- Commit subjects follow Conventional Commits: `chore(conventions): example`.

## Command map

| Task | Command |
|---|---|
| Install node toolchain | `npm ci` |
| The gate (before every push) | `npm run check` |
| Rust tests only | `cargo test --manifest-path src-tauri/Cargo.toml` |
| Install-script pin check (network) | `npm run pin:check` |
| Dependency audit parity (network / `--offline`) | `scripts/deps.sh` |
| Enable hooks | `git config core.hooksPath .githooks` |
| Ship work | push task branch, `gh pr create --base dev` |

## Where the docs live

- `AGENTS.md` (this file) → [CONVENTIONS.md](CONVENTIONS.md) (canonical rule
  book) → [ROADMAP.md](ROADMAP.md) (plans and working notes).
- [README.md](README.md) is product facts only.

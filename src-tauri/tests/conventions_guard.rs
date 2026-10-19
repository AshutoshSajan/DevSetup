//! Contract tests: pin the conventions/enforcement stack in place.
//!
//! These live in the project's real test suite on purpose: if any file below
//! is deleted, weakened or reworded away from its contract, `cargo test`
//! fails — in CI (guard workflows run on trunks, deps-audit on shared files)
//! and locally (the pre-push gate runs `npm run check`, which runs these).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The repository root (this Cargo manifest lives in `<repo>/src-tauri`).
fn root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p
}

fn rd(rel: &str) -> String {
    let path = root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {rel} ({path:?}): {e}"))
}

fn must_have(text: &str, what: &str, needle: &str) {
    assert!(
        text.contains(needle),
        "contract violated in {what}: expected the exact text {needle:?}"
    );
}

fn must_not_have(text: &str, what: &str, needle: &str) {
    assert!(
        !text.contains(needle),
        "contract violated in {what}: must NOT contain {needle:?}"
    );
}

// ---------- hooks ----------

#[test]
fn hooks_exist_parse_and_are_executable() {
    for name in ["pre-push", "commit-msg", "post-merge"] {
        let rel = format!(".githooks/{name}");
        let path = root().join(&rel);
        assert!(path.exists(), "{rel} is missing");
        // Must be executable in the repo, not only locally.
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_ne!(mode & 0o111, 0, "{rel} is not executable (chmod +x it)");
        // Must be valid bash.
        let st = Command::new("bash").arg("-n").arg(&path).status().unwrap();
        assert!(st.success(), "{rel} does not parse with bash -n");
    }
}

#[test]
fn pre_push_contract() {
    let h = rd(".githooks/pre-push");
    // Refuses trunks from argv AND stdin, with a boundary regex.
    must_have(&h, "pre-push", "refs/heads/(main|dev)([[:space:]]|$)");
    must_have(&h, "pre-push", "REFUSED: pushes to the protected trunks");
    must_have(&h, "pre-push", "gh pr create --base dev");
    // Git plumbing env is cleaned before the gate runs (build tools that
    // shell out to `git` would otherwise describe the wrong repository).
    must_have(
        &h,
        "pre-push",
        "unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_PREFIX",
    );
    // The gate is the project's own check target.
    must_have(&h, "pre-push", "npm run check");
}

#[test]
fn pre_push_refuses_trunk_payloads() {
    let hook = root().join(".githooks/pre-push");
    // <local-ref> <local-sha> <remote-ref> <remote-sha> — normal push, forced
    // push, and the deletion form (local sha is all zeros). All must be
    // refused while naming the rule.
    let lines = [
        "refs/heads/dev 1111111111111111111111111111111111111111 refs/heads/dev 2222222222222222222222222222222222222222",
        "refs/heads/main 3333333333333333333333333333333333333333 refs/heads/main 4444444444444444444444444444444444444444",
        "refs/heads/dev 0000000000000000000000000000000000000000 refs/heads/dev 2222222222222222222222222222222222222222",
    ];
    for line in lines {
        let payload_path = std::env::temp_dir().join(format!(
            "devsetup-prepush-{}-{}.txt",
            std::process::id(),
            chronoless()
        ));
        fs::write(&payload_path, format!("{line}\n")).unwrap();
        // Opened read-only so the hook's buffered read starts at offset 0.
        let stdin = fs::File::open(&payload_path).unwrap();
        let out = Command::new("bash")
            .arg(&hook)
            .stdin(Stdio::from(stdin))
            .output()
            .unwrap_or_else(|e| panic!("cannot run pre-push with payload '{line}': {e}"));
        assert!(
            !out.status.success(),
            "pre-push accepted a trunk payload: {line}"
        );
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            stderr.contains("REFUSED"),
            "pre-push must name the rule on refusal, payload '{line}'\n{stderr}"
        );
        let _ = fs::remove_file(&payload_path);
    }
}

#[test]
fn pre_push_lets_task_branches_through_the_trunk_check() {
    // A task branch payload must NOT trip the trunk regex (boundary check:
    // mainfoo / dev/x / feat/… must be allowed to reach the gate step).
    // We only execute the trunk-check portion here: running the full hook
    // would run the whole project gate, which is exercised live in CI and by
    // an actual push. So: verify the hook texts contain the allow shapes?
    // Empirical, cheap version: patch the hook in memory so the gate is a
    // no-op, run it, and require exit 0.
    let hook = root().join(".githooks/pre-push");
    let src = fs::read_to_string(&hook).unwrap();
    let patched = src.replace("npm run check", "true");
    assert_ne!(patched, src, "gate line disappeared from pre-push");

    let payloads = [
        "refs/heads/feat/cleaner 1111111111111111111111111111111111111111 refs/heads/feat/cleaner 2222222222222222222222222222222222222222",
        "refs/heads/mainfoo 3333333333333333333333333333333333333333 refs/heads/mainfoo 4444444444444444444444444444444444444444",
        "refs/heads/devtools 0000000000000000000000000000000000000000 refs/heads/devtools 2222222222222222222222222222222222222222",
        "refs/heads/chore/x 0000000000000000000000000000000000000000 refs/heads/chore/x 2222222222222222222222222222222222222222",
    ];
    for line in payloads {
        let payload_path = std::env::temp_dir().join(format!(
            "devsetup-prepush-ok-{}-{}.txt",
            std::process::id(),
            chronoless()
        ));
        fs::write(&payload_path, format!("{line}\n")).unwrap();
        let out = Command::new("bash")
            .arg("-c")
            .arg(&patched)
            .stdin(Stdio::from(fs::File::open(&payload_path).unwrap()))
            .current_dir(root()) // git rev-parse must work like in a real hook
            .output()
            .unwrap_or_else(|e| panic!("cannot run pre-push with payload '{line}': {e}"));
        assert!(
            out.status.success(),
            "a task-branch payload must pass the trunk check (gate replaced by true), payload '{line}'\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        let _ = fs::remove_file(&payload_path);
    }
}

#[test]
fn commit_msg_contract() {
    let hook = root().join(".githooks/commit-msg");
    let msgpath = std::env::temp_dir().join(format!("devsetup-cm-{}-{}.txt", std::process::id(), chronoless()));

    // Subjects that must PASS.
    for subject in [
        "Merge pull request #5 from AshutoshSajan/feat/x",
        "Merge branch 'dev' into main",
        "Revert \"feat: too much\"",
        "fixup! feat: finish it",
        "chore(conventions): pin the stack",
        "feat: add the cleaner tab",
        "docs(readme): point to agent docs",
        "fix(exec)!: drop the broken retry",
    ] {
        fs::write(&msgpath, format!("{subject}\n\nbody\n")).unwrap();
        let out = run_bash(&hook, &msgpath);
        // run_bash returns the raw output; assert success
        assert!(
            out.0.success(),
            "commit-msg must accept '{subject}'\nstderr: {}",
            out.1
        );
    }
    // Subjects that must be rejected.
    for subject in [
        "update files",
        "chore:no space after colon",
        "WIP: halfway there",
        "feat without colon",
        ":empty type",
    ] {
        fs::write(&msgpath, format!("{subject}\n")).unwrap();
        let out = run_bash(&hook, &msgpath);
        assert!(
            !out.0.success(),
            "commit-msg must reject '{subject}'\nstderr: {}",
            out.1
        );
    }
    let _ = fs::remove_file(&msgpath);

    let h = fs::read_to_string(&hook).unwrap();
    must_have(&h, "commit-msg", "^(build|chore|ci|docs|feat|fix|perf|refactor|revert|style|test)");
}

/// Runs a hook with a message file argument; returns (status, stderr).
fn run_bash(hook: &Path, msg_file: &Path) -> (std::process::ExitStatus, String) {
    let out = Command::new("bash")
        .arg(hook)
        .arg(msg_file)
        .output()
        .unwrap_or_else(|e| panic!("cannot run {hook:?}: {e}"));
    (out.status, String::from_utf8_lossy(&out.stderr).into_owned())
}

/// A millisecond-ish tick so parallel test files do not collide.
fn chronoless() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() % 1_000_000
}

#[test]
fn post_merge_contract() {
    let h = rd(".githooks/post-merge");
    must_have(&h, "post-merge", "git fetch --prune origin");
}

// ---------- workflows ----------

#[test]
fn guard_workflows_pin_trunk_integrity() {
    for (file, branch, name) in [
        (".github/workflows/guard-dev.yml", "dev", "Guard dev"),
        (".github/workflows/guard-main.yml", "main", "Guard main"),
    ] {
        let w = rd(file);
        must_have(&w, file, &format!("branches: [{branch}]"));
        must_have(&w, file, name);
        must_have(&w, file, "permissions: {}");
        must_have(&w, file, "contents: read");
        // Payload-only: no checkout, no repo material — seconds-cheap jobs.
        must_not_have(&w, file, "actions/checkout");
        // Merge-commit-ness, force and deletion facts from the payload,
        // with a REST fallback when the payload ships no parent list
        // (observed for merges made in the GitHub UI).
        must_have(&w, file, "'.head_commit // null'");
        must_have(&w, file, "'.forced // false'");
        must_have(&w, file, "'.head_commit.parents // empty | length'");
        must_have(&w, file, "repos/$REPO/commits/$SHA");
        must_have(&w, file, "-lt 2");
    }
}

#[test]
fn prune_workflow_contract() {
    let w = rd(".github/workflows/prune-branch.yml");
    must_have(&w, "prune-branch.yml", "types: [closed]");
    must_have(&w, "prune-branch.yml", "branches: [dev, main]");
    must_have(&w, "prune-branch.yml", "github.event.pull_request.merged == true");
    must_have(
        &w,
        "prune-branch.yml",
        "github.event.pull_request.head.repo.full_name == github.repository",
    );
    // Trunks are never deletable.
    must_have(&w, "prune-branch.yml", "case \"$HEAD_REF\" in\n            main|dev)");
    // Only allowlisted namespaces.
    must_have(&w, "prune-branch.yml", "feat/*|fix/*|chore/*|ci/*|docs/*|perf/*|refactor/*|test/*|release/*|dependabot/*");
    // Ancestry proof via the authoritative compare API (no shallow guesses),
    // and deletion only through it.
    must_have(&w, "prune-branch.yml", "repos/$REPO/compare/$BASE_REF...$HEAD_SHA");
    must_have(&w, "prune-branch.yml", "behind|identical");
    must_have(&w, "prune-branch.yml", "repos/$REPO/git/refs/heads/$HEAD_REF");
    // Payload-only: pruner must not need a checkout at all.
    must_not_have(&w, "prune-branch.yml", "actions/checkout");
}

#[test]
fn dependabot_targets_the_integration_trunk() {
    let d = rd(".github/dependabot.yml");
    for eco in ["package-ecosystem: npm", "package-ecosystem: cargo", "package-ecosystem: github-actions"] {
        must_have(&d, "dependabot.yml", eco);
    }
    for t in ["interval: weekly", "target-branch: dev"] {
        must_have(&d, "dependabot.yml", t);
    }
}

#[test]
fn deps_audit_is_pinned_and_off_round() {
    let workflow = rd(".github/workflows/deps-audit.yml");
    let script = rd("scripts/deps.sh");

    // Weekly cron on an off-round minute.
    must_have(&workflow, "deps-audit.yml", "'23 4 * * 1'");
    // Pinned scanner: version AND checksum, verified with sha256sum -c -.
    let sha = "160c6253e2922e9e0a6a9771c5c4c40a4b3c6090b34a3cdd5e80c96cc2e4218b";
    must_have(&workflow, "deps-audit.yml", "OSV_VERSION=v2.7.0");
    must_have(&workflow, "deps-audit.yml", "sha256sum -c -");
    must_have(&workflow, "deps-audit.yml", sha);
    // It checks out the repo, so it must hold real read permissions
    // (payload-only `permissions: {}` breaks its checkout).
    must_have(&workflow, "deps-audit.yml", "contents: read");
    // Both files must pin the same scanner binary (drift is the failure mode).
    must_have(&script, "scripts/deps.sh", "OSV_VERSION=v2.7.0");
    must_have(&script, "scripts/deps.sh", sha);
    // Scans both lockfiles through the reviewed exemption file; npm audit
    // (prod scope) and pin staleness round it out.
    must_have(&workflow, "deps-audit.yml", "--config=.osv-scanner.toml");
    must_have(&workflow, "deps-audit.yml", "--lockfile=package-lock.json");
    must_have(&workflow, "deps-audit.yml", "--lockfile=src-tauri/Cargo.lock");
    must_have(&workflow, "deps-audit.yml", "npm audit --omit=dev");
    must_have(&workflow, "deps-audit.yml", "node scripts/pin-scripts.mjs --check");
    must_have(&script, "scripts/deps.sh", "--config=.osv-scanner.toml");
    script_has_gate(&script);
}

/// Exemptions are data with a receipt: version-scoped so they lapse when the
/// package moves, each with a reason, and mirrored by the CONVENTIONS policy
/// (an unsound exemption is a violation, not a bypass).
#[test]
fn osv_exemptions_are_scoped_and_reasoned() {
    let cfg = rd(".osv-scanner.toml");

    for package in ["glib", "proc-macro-error", "unsafe-libyaml"] {
        must_have(&cfg, ".osv-scanner.toml", &format!("name = \"{package}\""));
    }
    // Every entry is scoped to the exact vulnerable version and carries a
    // reason (count one `version =` and one `reason =` per [[PackageOverrides]]).
    let entries = cfg.match_indices("[[PackageOverrides]]").count();
    assert_eq!(
        entries,
        3,
        "expected 3 documented overrides in .osv-scanner.toml, found {entries} - every exemption change is a reviewed PR"
    );
    assert_eq!(cfg.matches("version = \"").count(), 3, "each override must be version-scoped");
    assert_eq!(cfg.matches("reason = \"").count(), 3, "each override must carry a reason");
    // The three vulnerable versions currently exempted; bumping any of them
    // without a reviewed entry goes red again.
    for version in ["0.18.5", "1.0.4", "0.2.11"] {
        must_have(&cfg, ".osv-scanner.toml", &format!("version = \"{version}\""));
    }
    // Policy must be written down next to the mechanism.
    let conv = rd("CONVENTIONS.md");
    must_have(&conv, "CONVENTIONS.md", "Dependency exceptions");
}

fn script_has_gate(script: &str) {
    must_have(script, "scripts/deps.sh", "sha256sum -c -");
}
// ---------- docs ----------

#[test]
fn agents_md_carries_the_hard_rules_inline() {
    let a = rd("AGENTS.md");
    for rule in [
        "Never commit or push to the trunks (`dev`, `main`)",
        "gh pr create --base dev",
        "Never drop, delete or truncate databases",
        "Never rewrite shared history",
        "Never commit secrets",
        "Regenerate; never hand-edit",
        "Run the gate before every push:",
        "git config core.hooksPath .githooks",
        "linked worktree",
        "Actions that cannot be undone are not options",
    ] {
        must_have(&a, "AGENTS.md", rule);
    }
}

#[test]
fn claude_md_imports_agents_md_only() {
    let c = rd("CLAUDE.md");
    must_have(&c, "CLAUDE.md", "@AGENTS.md");
    // No duplicate rule text — the import is the mechanism.
    must_not_have(&c, "CLAUDE.md", "Never commit");
}

#[test]
fn readme_is_product_only_and_links_the_conventions() {
    let r = rd("README.md");
    must_have(&r, "README.md", "CONVENTIONS.md");
    must_have(&r, "README.md", "AGENTS.md");
    must_have(&r, "README.md", "ROADMAP.md");
    // Retired tiers must not reappear in the product readme.
    must_not_have(&r, "README.md", "staging");
    must_not_have(&r, "README.md", "sandbox");
}

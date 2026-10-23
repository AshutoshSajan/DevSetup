# DevSetup roadmap

Status: `[x]` done, `[~]` started, `[ ]` planned. Features are built roughly in this order.

## Done
- [x] Role wizard with 11 roles and about 130 tools (`manifest/tools.yaml`)
- [x] Tool list follows the chosen roles, search, "Show all", select all / deselect all
- [x] Installed-tool detection (dpkg, Flatpak, PATH) and automatic requirements
- [x] System / Light / Dark theme

## Phase 1: a real installer
- [x] Install executor: one `pkexec` helper for all apt work, user-level steps for Flatpak, mise, npm, pipx and install scripts, live logs per step
- [x] State file: remembers what DevSetup installed; only those tools can be removed
- [x] Plan review screen: installs, removals and left-alone tools, the exact commands in execution order (built from the same code that runs them), apt package sizes, and a copy button. Pre-flight checks add the total download and disk estimate
- [x] Pre-flight checks in the review screen: supported distro, pkexec, curl, reachable hosts, free disk space, apt package lookup with size estimate
- [ ] Secure Boot warning (needed once driver tools exist, Phase 3)
- [x] Cancel a running install (stops user-level steps; the root apt step finishes first, since stopping it is unsafe) and retry failed or cancelled steps, one by one or all at once
- [x] Remove tools DevSetup installed with apt, Flatpak, mise, npm and pipx, and install-script tools that define an `uninstall` command. Dependents go first, and a tool another remaining tool needs is kept
- [x] Pinned install scripts: checksum verification before a script runs, strict mode, optional version pins for mise, npm and pipx tools, and `npm run pin` to generate the hashes. All eight install scripts carry committed pins, and the weekly `deps-audit` workflow runs the stale-pin check
- [ ] Polkit policy and `.deb` packaging for the root helper (`packaging/`)
- [~] Unit and integration tests for plan and executor: the planning rules, the install and remove commands and their ordering are unit-tested. Still to do: an end-to-end run against a throwaway root

## Phase 2: features that set it apart
- [x] Save and share setups: export the selection as `devsetup.yaml` (file or copied text), load it from a file, pasted text or a GitHub link (`owner/repo`, repo, blob or gist URL). Loading adds to your selection; unknown tools are reported
- [x] Scan this machine: detect which tools from the list are installed, suggest roles, and save the result as a setup file (back up before a reinstall). Flatpak apps, npm globals and pipx apps outside the list are recorded as comments. Still to do: scan apt manual packages and mise tools
- [x] Cleaner tab: unused apt packages (with the list shown first), apt download cache, Docker prune (volumes never touched) and unused images, unused Flatpak runtimes, and npm, pip, mise and Cargo caches, with the estimated space each frees. Still to do: journal logs, Flatpak sizes, per-folder build caches
- [x] Update all: system packages (apt upgrade), Flatpak, mise, npm globals, pipx, and the own updaters of script-installed tools (mise, Bun, Deno, uv, rustup, Claude Code, OpenCode). Counts and package names are shown first where they can be checked
- [x] Doctor: checks that installed commands run, that tools in your home folder are on a new terminal's PATH, Docker service and group, Git identity, Flathub, the apt package database, disk space and pending restarts, with fixes (one click, password prompt, or copy-paste lines)

## Phase 3: post-install setup
- [x] Git and GitHub wizard: name, email and default branch, an ed25519 SSH key (never overwrites an existing one), GitHub sign-in through the browser with the GitHub CLI, adding the key to GitHub, and commit signing with the SSH key. Still to do: passphrase through an agent prompt, several keys per host, GPG signing
- [x] Shell setup: a marked block in ~/.zshrc or ~/.bashrc with history defaults, completion, Starship, zsh-autosuggestions, zsh-syntax-highlighting, zoxide, fzf, direnv, mise and aliases, with preview, one-time backup and removal, plus switching the login shell to zsh. Still to do: fish, installing the missing tools in one click, oh-my-zsh style frameworks
- [ ] VS Code extension packs per role
- [ ] System tweaks: Docker group and service, file-watcher limits, Flathub, firewall, timezone
- [ ] Driver helpers: NVIDIA drivers, CUDA, container toolkit for GPU Docker, Android udev rules for `adb`

## Phase 4: Stacks tab (disposable Docker services)
- [ ] Stack cards per project: start, stop, destroy (containers, volumes, network)
- [ ] Services: Redis, MongoDB, Postgres, MySQL, Kafka, RabbitMQ, MinIO, Elasticsearch, Mailpit, with GUI sidecars
- [ ] Ready-made combos: MERN, Django + Postgres + Redis, LAMP
- [ ] Port-conflict detection, `.env` export, volume backup and restore, idle-stack reminders
- [ ] Connection strings with copy buttons, disk usage per stack

## Phase 5: later
- [ ] Command-line version: `devsetup apply devsetup.yaml` for servers, CI and WSL
- [ ] Fedora, Arch and macOS support
- [ ] Remote, signed manifest updates so new tools arrive without an app release
- [ ] Tool detail panel with description, size and version picker
- [ ] Ollama and other installers that need sudo

## Known issues

### Security-clearing dependency migrations (exemptions in `.osv-scanner.toml`)
- [ ] Tauri 3 / gtk-rs 0.20 migration — clears the `glib` exemption
  (RUSTSEC-2024-0429) and replays the whole local-GTK install path on the
  new stack. Largest migration on this list; plan as `feat/tauri-3`.
- [x] vite 8 migration (2026-10-19): `^5` → `^8.3.4` shipped via the
  `update-vite-version` PR. Vite 8's rolldown core dropped esbuild entirely,
  so the `vite` and `esbuild` exemptions had nothing left to match and were
  removed from `.osv-scanner.toml`. Verified: `vite build`, `tsc --noEmit`
  (typescript 7 native) and a dev-server boot on :1420 are green — do one
  full `tauri dev` session on first real use to close the old dev-flow
  concern completely.
- [ ] Unmaintained-crate chain cleanup — `proc-macro-error` and
  `unsafe-libyaml` have no fixed release; the exemption lapses when
  dependencies stop pulling them (watch Dependabot majors, serde_yaml
  replacements).

- The Doctor terminal-PATH check starts your `$SHELL` interactively (bash, zsh, sh, dash or fish) and is an approximation: shell frameworks that print output or ask questions can make it report "could not check".
- Upgrading Node through mise installs a new Node version, and global npm packages (Codex CLI, Gemini CLI, pnpm) from the old version are not carried over. Install them again afterwards; DevSetup then shows them as missing.
- Apt package names and Flatpak IDs are not yet verified against Debian 13 (for example `docker-compose`, `redis-server`, `starship`, `eza`). The pre-flight check now warns about apt names missing from your sources; Flatpak IDs are not checked yet.
- "System" theme may not follow the Plasma theme in some WebKitGTK versions; pick Light or Dark.
- Uninstall commands for install-script tools are my best reading of the vendor documentation and are not verified on a real machine. Ollama has none (it installs system-wide) and must be removed manually.
- Removal leaves configuration folders such as `~/.claude` and `~/.config/opencode` in place.

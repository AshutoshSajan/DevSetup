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
- [~] Pinned install scripts: checksum verification before a script runs, strict mode, optional version pins for mise, npm and pipx tools, and `npm run pin` to generate the hashes. Still to do: generate and commit the real hashes (needs internet) and re-pin on a schedule in CI
- [ ] Polkit policy and `.deb` packaging for the root helper (`packaging/`)
- [ ] Unit and integration tests for plan and executor

## Phase 2: features that set it apart
- [x] Save and share setups: export the selection as `devsetup.yaml` (file or copied text), load it from a file, pasted text or a GitHub link (`owner/repo`, repo, blob or gist URL). Loading adds to your selection; unknown tools are reported
- [x] Scan this machine: detect which tools from the list are installed, suggest roles, and save the result as a setup file (back up before a reinstall). Flatpak apps, npm globals and pipx apps outside the list are recorded as comments. Still to do: scan apt manual packages and mise tools
- [x] Cleaner tab: unused apt packages (with the list shown first), apt download cache, Docker prune (volumes never touched) and unused images, unused Flatpak runtimes, and npm, pip, mise and Cargo caches, with the estimated space each frees. Still to do: journal logs, Flatpak sizes, per-folder build caches
- [ ] Update all: apt, Flatpak, mise, npm globals and script-installed tools in one go
- [ ] Doctor: checks tools run, PATH is right, user is in the `docker` group, and suggests fixes

## Phase 3: post-install setup
- [ ] Git and GitHub wizard: name and email, SSH key, `gh auth`, commit signing
- [ ] Shell setup: zsh with prompt and plugins, aliases and history defaults
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
- Apt package names and Flatpak IDs are not yet verified against Debian 13 (for example `docker-compose`, `redis-server`, `starship`, `eza`). The pre-flight check now warns about apt names missing from your sources; Flatpak IDs are not checked yet.
- "System" theme may not follow the Plasma theme in some WebKitGTK versions; pick Light or Dark.
- Uninstall commands for install-script tools are my best reading of the vendor documentation and are not verified on a real machine. Ollama has none (it installs system-wide) and must be removed manually.
- Removal leaves configuration folders such as `~/.claude` and `~/.config/opencode` in place.

<p align="center"><img src="docs/logo.png" width="128" alt="DevSetup logo"></p>

<h1 align="center">DevSetup</h1>

A desktop app (Tauri 2) that sets up a fresh Linux machine for development.
Pick your role, tick the tools, review the plan and install. It also removes what it installed.

**Status:** early development. The Debian/Ubuntu install flow is implemented but not yet tested on many machines.
Try it on a spare VM first.

## Features

Legend: ✅ available, 🚧 partly done, 🗓 planned. Full details in [`ROADMAP.md`](ROADMAP.md).

**Available now**
- ✅ Role wizard: 11 roles and about 130 tools, defined in `manifest/tools.yaml`
- ✅ Tool list follows your roles, with search, "Show all", select all / deselect all
- ✅ Installed-tool detection (dpkg, Flatpak, PATH) and automatic requirements
- ✅ System / Light / Dark theme
- ✅ Install executor: one password prompt for apt, user-level steps for Flatpak, mise, npm, pipx and install scripts, live logs
- ✅ State file: only tools DevSetup installed can be removed
- ✅ Pre-flight checks: distro, pkexec, curl, reachable hosts, free disk space, apt package lookup and size estimate
- ✅ Cancel a running install and retry failed or cancelled steps (the apt step cannot be interrupted safely, so cancel waits for it)
- ✅ Removal of tools DevSetup installed: apt, Flatpak, mise, npm, pipx, and install-script tools with an uninstall command (dependents first; a tool another remaining tool needs is kept)
- 🚧 Install-script pinning: each script is checked against a SHA-256 before it runs, with an optional strict mode and `npm run pin` to generate the hashes (the hashes themselves are not committed yet)
- 🚧 Plan review: lists installs, removals and left-alone tools (exact commands and per-tool sizes still to come)

**Phase 1: a real installer**
- 🗓 Secure Boot warning (once driver tools exist)
- 🗓 Polkit policy and `.deb` packaging for the root helper
- 🗓 Unit and integration tests for the plan and executor

**Phase 2: features that set it apart**
- 🗓 Save and share setups: export and import `devsetup.yaml`
- 🗓 Scan this machine and generate a setup file
- 🗓 Cleaner tab: `apt autoremove`, Docker prune, unused Flatpak runtimes, language caches, with space shown first
- 🗓 Update all: apt, Flatpak, mise, npm globals, script-installed tools
- 🗓 Doctor: check tools, PATH and the `docker` group, with fixes

**Phase 3: post-install setup**
- 🗓 Git and GitHub wizard: name and email, SSH key, `gh auth`, commit signing
- 🗓 Shell setup: zsh with prompt and plugins, aliases and history defaults
- 🗓 VS Code extension packs per role
- 🗓 System tweaks: Docker group and service, file-watcher limits, Flathub, firewall, timezone
- 🗓 Driver helpers: NVIDIA, CUDA, container toolkit for GPU Docker, Android udev rules for `adb`

**Phase 4: Stacks tab (disposable Docker services)**
- 🗓 Stack cards per project: start, stop, destroy (containers, volumes, network)
- 🗓 Services: Redis, MongoDB, Postgres, MySQL, Kafka, RabbitMQ, MinIO, Elasticsearch, Mailpit, with GUI sidecars
- 🗓 Ready-made combos: MERN, Django + Postgres + Redis, LAMP
- 🗓 Port-conflict detection, `.env` export, volume backup and restore, idle-stack reminders
- 🗓 Connection strings with copy buttons and disk usage per stack

**Phase 5: later**
- 🗓 Command-line version: `devsetup apply devsetup.yaml`
- 🗓 Fedora, Arch and macOS support
- 🗓 Signed remote manifest updates
- 🗓 Tool detail panel with description, size and version picker
- 🗓 Installers that need sudo, such as Ollama

## Run it (Debian/Ubuntu)
    sudo apt install build-essential libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    npm install
    npm run tauri dev

Run the Rust tests with `cargo test` inside `src-tauri`.
App icons live in `src-tauri/icons/` and are generated from `docs/app-icon.png` (source: `docs/logo.svg`). To change the logo, edit the SVG, export a 1024 px PNG to `docs/app-icon.png` and run `npm run icons`.

## How installing works
1. Pick roles and tools. Tools needed by a ticked tool are ticked automatically.
2. "Review plan" shows what will be installed, removed or left alone, and runs the pre-flight checks.
   A failed check disables Install; warnings do not.
3. System packages (apt) run in one `pkexec` call, so you type your password once.
   The root helper (`devsetup --helper apt +id -id`) accepts only tool ids, looks each up in the built-in manifest
   and refuses anything that is not an apt tool.
4. Flatpak apps, mise tools, npm and pipx packages and install scripts run as your own user, one by one, with live logs.
5. Cancel stops the step that is running and skips the rest. System packages (apt) run as root and cannot be interrupted safely, so cancel takes effect once they finish. Failed or cancelled steps can be retried one by one or all at once.
6. DevSetup records what it installed in `~/.local/share/devsetup/state.json` and removes only those tools. (Builds from when the app was called FlashSetup used `~/.local/share/flashsetup/`; that file is still read if the new one does not exist.)

For packaged installs, copy `packaging/dev.devsetup.helper.policy` to `/usr/share/polkit-1/actions/`.

## Removing tools
Untick a tool that DevSetup installed and it appears under "Will remove" in the review. Tools you installed yourself are never removed.
- Removal runs user-level tools first (dependents before what they need, for example Codex before Node), then apt packages in the single `pkexec` call.
- A tool is kept, with a warning, while another tool that stays installed needs it.
- How each kind is removed: Flatpak `uninstall --user`, mise `unuse` and `uninstall --all`, `npm uninstall -g`, `pipx uninstall`, apt `remove`.
  Install-script tools use the `uninstall:` command in the manifest; tools without one show "remove manually".
- Configuration folders (for example `~/.claude`) are left in place.

## Install-script pinning
Some tools install by running a script from the internet (mise, Bun, Deno, uv, rustup, Claude Code, OpenCode, Ollama).
- A tool can carry `sha256: "<hash>"` in `manifest/tools.yaml`. DevSetup downloads the script, compares its SHA-256 and
  **refuses to run it on a mismatch**. Scripts without a pin run as downloaded and show a warning in the checks.
- The review screen has a strict option: "Only run install scripts that have a pinned checksum".
- `npm run pin` downloads every script (with the same `curl` call the app uses) and writes the hashes into the manifest.
  `npm run pin:check` only reports scripts that changed upstream (exit code 1), so CI can flag stale pins.
  Review the diff of a changed script before committing its new hash.
- Trade-off: vendors update their scripts often, so a pin can go stale and the install fails with a checksum mismatch until the
  pin is refreshed. That is the intended behavior, and it is why re-pinning should run on a schedule.
- `mise`, `npm` and `pipx` tools accept an optional `version:` in the manifest (for example `{ type: mise, tool: "node", version: "22" }`).

## Tools and roles
`manifest/tools.yaml` holds roles and tools. A tool's `roles` list decides when it is shown:
a role id, `all` (always shown and pre-ticked) or `any` (shown once any role is picked).
GUI apps use Flatpak, CLI tools use apt, mise, npm, pipx or an install script.
`cargo test` checks that the manifest parses and that tool ids, roles and `requires` are consistent.

## Layout
- `manifest/tools.yaml`: roles and tools
- `src-tauri/src/`: `manifest.rs`, `system.rs` (distro and installed detection), `plan.rs`, `preflight.rs`, `exec.rs` (executor), `helper.rs` (root helper), `state.rs`, `lib.rs` (commands)
- `src/api.ts`: the only frontend file that talks to Tauri

// The only file that knows about Tauri. Swap this adapter to move to another shell.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
export type Distro = { id: string; name: string; version: string };
export type Role = { id: string; name: string; description: string };
export type Tool = { id: string; name: string; group: string; check: string; roles: string[]; requires: string[]; method: { type: string; url?: string; sha256?: string | null; version?: string | null }; installed: boolean; managed: boolean };
export type Manifest = { roles: Role[] };
export type Plan = { install: string[]; remove: string[]; left_alone: string[]; keep: string[]; warnings: string[] };
export type Step = { id: string; state: string; message: string };
export type LogLine = { id: string; stream: string; line: string };
export type Check = { id: string; label: string; status: "ok" | "warn" | "fail"; detail: string };
export type StepInfo = { id: string; name: string; phase: "remove" | "apt" | "install"; command: string; notes: string[] };
export type Details = { steps: StepInfo[]; skipped: string[] };
export type Imported = { name: string; roles: string[]; tools: string[]; unknown_roles: string[]; unknown_tools: string[] };
export type Found = { id: string; name: string; group: string; source: string; managed: boolean };
export type RoleHit = { id: string; name: string; installed: number; total: number; suggested: boolean };
export type Scan = { system: string; found: Found[]; roles: RoleHit[]; extras: { flatpak: string[]; npm: string[]; pipx: string[] } };
export type CleanItem = { id: string; name: string; description: string; command: string; bytes: number | null; detail: string; needs_root: boolean; risk: "safe" | "review"; available: boolean; reason: string };
export type UpdateItem = { id: string; name: string; description: string; command: string; count: number | null; detail: string; needs_root: boolean; available: boolean; reason: string };
export type FixInfo = { id: string; label: string; kind: "run" | "root" | "copy"; command: string; warning: string };
export type Finding = { id: string; title: string; status: "ok" | "warn" | "fail" | "info"; detail: string; advice: string; fix: FixInfo | null };
export type KeyInfo = { path: string; fingerprint: string; public: string };
export type GitState = { git_installed: boolean; ssh_installed: boolean; gh_installed: boolean; git_version: string; name: string; email: string; default_branch: string; gh_user: string | null; keys: KeyInfo[]; signing: boolean; signing_key: string };
export type ShOpts = { starship: boolean; autosuggestions: boolean; syntax_highlighting: boolean; history: boolean; completion: boolean; zoxide: boolean; fzf: boolean; direnv: boolean; mise: boolean; aliases: boolean; git_aliases: boolean };
export type Avail = { starship: boolean; zoxide: boolean; fzf: boolean; direnv: boolean; mise: boolean; eza: boolean; bat: boolean; fd: boolean; autosuggestions: boolean; syntax_highlighting: boolean };
export type RcInfo = { shell: string; path: string; exists: boolean; has_block: boolean; damaged: string | null };
export type ShellState = { login_shell: string; env_shell: string; zsh_installed: boolean; bash_installed: boolean; files: RcInfo[]; available: Avail };
export type ShPreview = { path: string; exists: boolean; had_block: boolean; block: string; warnings: string[] };
export type VsExt = { id: string; name: string };
export type VsPack = { id: string; name: string; description: string; roles: string[]; extensions: VsExt[] };
export type VsEditor = { id: string; name: string; installed: string[] };
export type VsState = { editors: VsEditor[]; packs: VsPack[] };
export type Done = { ok: boolean; failed: number; cancelled: boolean };
export const api = {
  system: () => invoke<Distro>("get_system"),
  manifest: () => invoke<Manifest>("get_manifest"),
  tools: () => invoke<Tool[]>("list_tools"),
  plan: (selected: string[]) => invoke<Plan>("make_plan", { selected }),
  preflight: (install: string[], remove: string[], strict: boolean) => invoke<Check[]>("preflight", { install, remove, strict }),
  details: (install: string[], remove: string[], strict: boolean) => invoke<Details>("plan_details", { install, remove, strict }),
  exportSetup: (name: string, roles: string[], tools: string[], notes: string[] = []) => invoke<string>("export_setup", { name, roles, tools, notes }),
  vscodeState: () => invoke<VsState>("vscode_state"),
  vscodeInstall: (editor: string, ids: string[]) => invoke<void>("vscode_install", { editor, ids }),
  shellState: () => invoke<ShellState>("shell_state"),
  shellPreview: (shell: string, opts: ShOpts) => invoke<ShPreview>("shell_preview", { shell, opts }),
  shellApply: (shell: string, opts: ShOpts) => invoke<string>("shell_apply", { shell, opts }),
  shellRemove: (shell: string) => invoke<string>("shell_remove", { shell }),
  setLoginShell: (shell: string) => invoke<void>("set_login_shell", { shell }),
  gitState: () => invoke<GitState>("git_state"),
  gitSaveIdentity: (name: string, email: string, defaultMain: boolean) => invoke<void>("git_save_identity", { name, email, defaultMain }),
  gitGenerateKey: (email: string, passphrase: string) => invoke<string>("git_generate_key", { email, passphrase }),
  gitTestSsh: () => invoke<{ ok: boolean; output: string }>("git_test_ssh"),
  gitAddKey: (path: string, kind: "authentication" | "signing") => invoke<string>("git_add_key", { path, kind }),
  gitSetupCredentials: () => invoke<string>("git_setup_credentials"),
  gitSetSigning: (enable: boolean, key: string, email: string) => invoke<void>("git_set_signing", { enable, key, email }),
  openUrl: (which: "github-device" | "github-ssh") => invoke<void>("open_url", { which }),
  ghLogin: (protocol: "ssh" | "https") => invoke<void>("gh_login", { protocol }),
  scanDoctor: () => invoke<Finding[]>("scan_doctor"),
  runFix: (id: string) => invoke<void>("run_fix", { id }),
  confirm: (message: string) => ask(message, { title: "DevSetup", kind: "warning" }),
  scanUpdates: () => invoke<UpdateItem[]>("scan_updates"),
  runUpdates: (ids: string[]) => invoke<void>("run_updates", { ids }),
  scanClean: () => invoke<CleanItem[]>("scan_clean"),
  runClean: (ids: string[]) => invoke<void>("run_clean", { ids }),
  scanMachine: () => invoke<Scan>("scan_machine"),
  parseSetup: (text: string) => invoke<Imported>("parse_setup", { text }),
  readSetupFile: (path: string) => invoke<string>("read_setup_file", { path }),
  writeSetupFile: (path: string, content: string) => invoke<void>("write_setup_file", { path, content }),
  fetchSetup: (source: string) => invoke<string>("fetch_setup", { source }),
  pickSavePath: () => save({ defaultPath: "devsetup.yaml", filters: [{ name: "DevSetup setup", extensions: ["yaml", "yml"] }] }),
  pickOpenPath: () => open({ multiple: false, directory: false, filters: [{ name: "DevSetup setup", extensions: ["yaml", "yml"] }] }),
  run: (install: string[], remove: string[], strict: boolean) => invoke<void>("run_plan", { install, remove, strict }),
  cancel: () => invoke<void>("cancel_run"),
  onStep: (f: (s: Step) => void) => listen<Step>("run-step", e => f(e.payload)),
  onLog: (f: (l: LogLine) => void) => listen<LogLine>("run-log", e => f(e.payload)),
  onDone: (f: (d: Done) => void) => listen<Done>("run-done", e => f(e.payload)),
};

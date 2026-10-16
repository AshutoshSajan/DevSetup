// The only file that knows about Tauri. Swap this adapter to move to another shell.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
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
export type Done = { ok: boolean; failed: number; cancelled: boolean };
export const api = {
  system: () => invoke<Distro>("get_system"),
  manifest: () => invoke<Manifest>("get_manifest"),
  tools: () => invoke<Tool[]>("list_tools"),
  plan: (selected: string[]) => invoke<Plan>("make_plan", { selected }),
  preflight: (install: string[], remove: string[], strict: boolean) => invoke<Check[]>("preflight", { install, remove, strict }),
  details: (install: string[], remove: string[], strict: boolean) => invoke<Details>("plan_details", { install, remove, strict }),
  exportSetup: (name: string, roles: string[], tools: string[], notes: string[] = []) => invoke<string>("export_setup", { name, roles, tools, notes }),
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

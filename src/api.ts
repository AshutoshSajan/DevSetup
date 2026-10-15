// The only file that knows about Tauri. Swap this adapter to move to another shell.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
export type Distro = { id: string; name: string; version: string };
export type Role = { id: string; name: string; description: string };
export type Tool = { id: string; name: string; group: string; check: string; roles: string[]; requires: string[]; method: { type: string; url?: string; sha256?: string | null; version?: string | null }; installed: boolean; managed: boolean };
export type Manifest = { roles: Role[] };
export type Plan = { install: string[]; remove: string[]; left_alone: string[]; keep: string[]; warnings: string[] };
export type Step = { id: string; state: string; message: string };
export type LogLine = { id: string; stream: string; line: string };
export type Check = { id: string; label: string; status: "ok" | "warn" | "fail"; detail: string };
export type Done = { ok: boolean; failed: number; cancelled: boolean };
export const api = {
  system: () => invoke<Distro>("get_system"),
  manifest: () => invoke<Manifest>("get_manifest"),
  tools: () => invoke<Tool[]>("list_tools"),
  plan: (selected: string[]) => invoke<Plan>("make_plan", { selected }),
  preflight: (install: string[], remove: string[], strict: boolean) => invoke<Check[]>("preflight", { install, remove, strict }),
  run: (install: string[], remove: string[], strict: boolean) => invoke<void>("run_plan", { install, remove, strict }),
  cancel: () => invoke<void>("cancel_run"),
  onStep: (f: (s: Step) => void) => listen<Step>("run-step", e => f(e.payload)),
  onLog: (f: (l: LogLine) => void) => listen<LogLine>("run-log", e => f(e.payload)),
  onDone: (f: (d: Done) => void) => listen<Done>("run-done", e => f(e.payload)),
};

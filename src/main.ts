import "./style.css";
import { api, Tool, Role, Plan, Imported, Scan, CleanItem, UpdateItem, Finding, FixInfo, GitState, ShellState, ShOpts, VsState, DriverReport, StacksState } from "./api";

const $ = <T extends HTMLElement = HTMLElement>(s: string) => document.querySelector(s) as T;
const el = (tag: string, cls = "", text = "") => { const e = document.createElement(tag); e.className = cls; e.textContent = text; return e; };

let tools: Tool[] = [];
let roleList: Role[] = [];
const roles = new Set<string>();
const overrides = new Map<string, boolean>(); // manual ticks and unticks win over role defaults
let showAll = false, query = "";
let lastPlan: Plan | null = null;
let strict = (() => { try { return localStorage.getItem("strict") === "1"; } catch { return false; } })();
const name = (id: string) => tools.find(t => t.id === id)?.name ?? id;
const label = (id: string) => { const t = tools.find(x => x.id === id); const v = t?.method.version; return (t?.name ?? id) + (v ? ` ${v}` : ""); };

// ---- theme: system | light | dark ----
type Theme = "system" | "light" | "dark";
function applyTheme(t: Theme) {
  const root = document.documentElement;
  if (t === "system") root.removeAttribute("data-theme"); else root.setAttribute("data-theme", t);
  try { localStorage.setItem("theme", t); } catch { /* storage unavailable */ }
  document.querySelectorAll<HTMLButtonElement>("#theme button").forEach(b => b.setAttribute("aria-pressed", String(b.dataset.t === t)));
}
function initTheme() {
  let t: Theme = "system";
  try { const s = localStorage.getItem("theme"); if (s === "light" || s === "dark") t = s; } catch { /* ignore */ }
  document.querySelectorAll<HTMLButtonElement>("#theme button").forEach(b => { b.onclick = () => applyTheme(b.dataset.t as Theme); });
  applyTheme(t);
}

// ---- selection ----
function withDeps(ids: Set<string>) {
  const byId = new Map(tools.map(t => [t.id, t]));
  let changed = true;
  while (changed) {
    changed = false;
    for (const id of [...ids]) for (const r of byId.get(id)?.requires ?? []) if (!ids.has(r)) { ids.add(r); changed = true; }
  }
  return ids;
}
function selection() {
  const s = new Set(tools.filter(t => t.installed || t.roles.includes("all") || t.roles.some(r => roles.has(r))).map(t => t.id));
  for (const [id, on] of overrides) on ? s.add(id) : s.delete(id);
  return withDeps(s);
}
function visible(t: Tool) {
  if (query) return `${t.name} ${t.id} ${t.group}`.toLowerCase().includes(query.toLowerCase());
  if (showAll || t.roles.includes("all")) return true;
  if (t.roles.includes("any")) return roles.size > 0;
  return t.roles.some(r => roles.has(r));
}

// ---- rendering ----
function renderRoles(list: Role[]) {
  const box = $("#roles"); box.replaceChildren();
  for (const r of list) {
    const card = el("label", "card"), cb = document.createElement("input");
    cb.type = "checkbox"; cb.checked = roles.has(r.id); card.classList.toggle("on", cb.checked);
    cb.onchange = () => { cb.checked ? roles.add(r.id) : roles.delete(r.id); card.classList.toggle("on", cb.checked); renderTools(); };
    const head = el("div", "row"); head.append(cb, el("b", "", r.name));
    card.append(head, el("div", "mu", r.description)); box.append(card);
  }
}
function renderTools() {
  const sel = selection(), vis = tools.filter(visible), box = $("#tools"); box.replaceChildren();
  for (const g of [...new Set(vis.map(t => t.group))]) {
    const card = el("div", "card"); card.append(el("h3", "", g));
    for (const t of vis.filter(x => x.group === g)) {
      const row = el("label", "row"), cb = document.createElement("input");
      cb.type = "checkbox"; cb.checked = sel.has(t.id);
      cb.onchange = () => { overrides.set(t.id, cb.checked); renderTools(); };
      const tag = t.installed ? (sel.has(t.id) ? "installed" : t.managed ? "will be removed" : "installed, left alone") : t.method.type;
      row.append(cb, el("span", "", t.name), el("span", "tag", tag)); card.append(row);
    }
    box.append(card);
  }
  $("#hint").textContent = !roles.size && !showAll && !query
    ? "Pick a role above to see more tools, or tick Show all."
    : `${vis.length} of ${tools.length} tools shown`;
  updatePlan(sel);
}
async function updatePlan(sel: Set<string>) {
  const p = await api.plan([...sel]);
  lastPlan = p;
  $<HTMLButtonElement>("#go").disabled = !(p.install.length || p.remove.length);
  $("#sum").textContent = `${p.install.length} to install · ${p.remove.length} to remove`;
  $("#warn").textContent = p.warnings.join(" · ");
}

$<HTMLInputElement>("#q").oninput = e => { query = (e.target as HTMLInputElement).value.trim(); renderTools(); };
$<HTMLInputElement>("#showall").onchange = e => { showAll = (e.target as HTMLInputElement).checked; renderTools(); };
$("#all").onclick = () => { tools.filter(visible).forEach(t => overrides.set(t.id, true)); renderTools(); };
$("#none").onclick = () => { tools.filter(visible).forEach(t => overrides.set(t.id, false)); renderTools(); };

// ---- review and run ----
const modal = $("#modal"), logEl = $("#log"), noteEl = $("#note"), mcancel = $<HTMLButtonElement>("#mcancel"), mgo = $<HTMLButtonElement>("#mgo");
let unlisten: Array<() => void> = [];
let reviewToken = 0, running = false;
let goAction: () => void = () => {};
const stopListening = () => { unlisten.forEach(f => f()); unlisten = []; };

function openReview() {
  if (!lastPlan) return;
  const p = lastPlan, body = $("#mbody");
  $("#mt").textContent = "Review plan"; body.replaceChildren(); logEl.hidden = true; noteEl.hidden = true;
  const section = (title: string, ids: string[]) => {
    if (!ids.length) return;
    body.append(el("h3", "", `${title} (${ids.length})`), el("div", "mu", ids.map(label).join(", ")));
  };
  section("Will install", p.install);
  section("Will remove (installed by DevSetup)", p.remove);
  section("Installed, left alone", p.left_alone);
  if (p.install.some(id => tools.find(t => t.id === id)?.method.type === "script")) {
    const lab = el("label", "row"), cb = document.createElement("input");
    cb.type = "checkbox"; cb.checked = strict;
    cb.onchange = () => { strict = cb.checked; try { localStorage.setItem("strict", strict ? "1" : "0"); } catch { /* ignore */ } openReview(); };
    lab.append(cb, el("span", "", "Only run install scripts that have a pinned checksum")); body.append(lab);
  }
  if (p.warnings.length) body.append(el("div", "st warn", p.warnings.join(" · ")));
  if ([...p.install, ...p.remove].some(id => tools.find(t => t.id === id)?.method.type === "apt"))
    body.append(el("div", "mu", "System packages need your password once."));
  goAction = () => startRun(p.install, p.remove);
  mgo.hidden = false; mgo.textContent = "Install"; mcancel.textContent = "Cancel"; mcancel.disabled = false;
  modal.hidden = false;

  // pre-flight checks: Install stays disabled until they finish and none has failed
  const token = ++reviewToken, box = el("div");
  body.append(el("h3", "", "Checks"), box); box.append(el("div", "mu", "Checking…")); mgo.disabled = true;
  const mark = { ok: "✓", warn: "!", fail: "✕" };
  // exact commands, in the order they will run
  const dbox = el("div");
  body.append(el("h3", "", "What will run"), dbox); dbox.append(el("div", "mu", "Loading commands…"));
  const phases = { remove: "1. Remove (as you)", apt: "2. System packages (needs your password)", install: "3. Install (as you)" };
  api.details(p.install, p.remove, strict).then(d => {
    if (token !== reviewToken || modal.hidden) return;
    dbox.replaceChildren();
    if (!d.steps.length && !d.skipped.length) { dbox.append(el("div", "mu", "Nothing to run.")); return; }
    const det = document.createElement("details"), sum = document.createElement("summary");
    sum.textContent = `Show exact commands (${d.steps.length} step${d.steps.length === 1 ? "" : "s"})`; det.append(sum);
    let last = "";
    for (const s of d.steps) {
      if (s.phase !== last) { det.append(el("h3", "", phases[s.phase])); last = s.phase; }
      const wrap = el("div", "cmd"); wrap.append(el("div", "", s.name), el("pre", "", s.command));
      for (const n of s.notes) wrap.append(el("div", "mu", n));
      det.append(wrap);
    }
    if (d.skipped.length) { det.append(el("h3", "", "Not run")); for (const n of d.skipped) det.append(el("div", "mu", n)); }
    const cp = el("button", "b2 mini", "Copy commands");
    cp.onclick = () => { navigator.clipboard?.writeText(d.steps.map(s => `# ${s.name}\n${s.command}`).join("\n\n")); cp.textContent = "Copied"; };
    det.append(cp);
    dbox.append(det);
  }).catch(e => { if (token === reviewToken) dbox.replaceChildren(el("div", "st warn", `! Commands could not be loaded: ${e}`)); });

  api.preflight(p.install, p.remove, strict).then(cs => {
    if (token !== reviewToken || modal.hidden) return;
    box.replaceChildren();
    for (const c of cs) {
      const r = el("div", "step"); r.append(el("span", "", c.label), el("span", "st " + c.status, `${mark[c.status]} ${c.detail}`)); box.append(r);
    }
    const failed = cs.some(c => c.status === "fail");
    if (failed) box.append(el("div", "mu", "Fix the failed checks to continue."));
    mgo.disabled = failed;
  }).catch(e => {
    if (token !== reviewToken) return;
    box.replaceChildren(el("div", "st warn", `! Checks could not run: ${e}`)); mgo.disabled = false;
  });
}

type RunOptions = {
  title: string;
  label: (id: string) => string;
  start: () => Promise<void>;
  retry?: (ids: string[]) => void;
  rootStep?: (id: string) => boolean;
  after?: () => Promise<void>;
  onLog?: (line: string) => void;
};

/** Shows live progress for a run (install or clean): steps, log, cancel, and retry of failed steps. */
async function runProgress(o: RunOptions) {
  const body = $("#mbody"), box = el("div"), rows = new Map<string, HTMLElement>(), states = new Map<string, string>();
  reviewToken++; modal.hidden = false;
  body.replaceChildren(box); logEl.textContent = ""; logEl.hidden = false; noteEl.hidden = true; $("#codebox").hidden = true;
  running = true;
  $("#mt").textContent = o.title; mgo.hidden = true; mcancel.textContent = "Cancel"; mcancel.disabled = false;
  const row = (id: string) => {
    let r = rows.get(id);
    if (!r) { r = el("div", "step"); r.append(el("span", "", o.label(id)), el("span", "st", "")); box.append(r); rows.set(id, r); }
    return r;
  };
  unlisten = await Promise.all([
    api.onStep(s => {
      const st = row(s.id).querySelector(".st") as HTMLElement; states.set(s.id, s.state);
      st.className = "st " + s.state; st.textContent = s.message ? `${s.state}: ${s.message}` : s.state;
      noteEl.hidden = !(s.state === "running" && !!o.rootStep?.(s.id));
    }),
    api.onLog(l => { logEl.textContent += `[${l.id}] ${l.line}\n`; logEl.scrollTop = logEl.scrollHeight; o.onLog?.(l.line); }),
    api.onDone(async d => {
      stopListening(); running = false; noteEl.hidden = true; $("#codebox").hidden = true;
      $("#mt").textContent = d.cancelled ? "Cancelled" : d.failed ? `Finished, ${d.failed} failed` : "Finished";
      mcancel.textContent = "Close"; mcancel.disabled = false;
      const unfinished = [...states].filter(([, st]) => st === "failed" || st === "cancelled").map(([id]) => id);
      if (unfinished.length && o.retry) {
        const retry = o.retry;
        goAction = () => retry(unfinished);
        mgo.textContent = `Retry unfinished (${unfinished.length})`; mgo.hidden = false; mgo.disabled = false;
        for (const id of unfinished) {
          const b = el("button", "b2 mini", "Retry"); b.onclick = () => retry([id]); rows.get(id)?.append(b);
        }
      }
      await o.after?.();
    }),
  ]);
  try { await o.start(); }
  catch (e) { stopListening(); running = false; logEl.textContent += `error: ${e}\n`; $("#mt").textContent = "Could not start"; mcancel.textContent = "Close"; mcancel.disabled = false; }
}

function startRun(install: string[], remove: string[]) {
  return runProgress({
    title: "Installing",
    label: name,
    start: () => api.run(install, remove, strict),
    retry: ids => { void startRun(ids.filter(i => !remove.includes(i)), ids.filter(i => remove.includes(i))); },
    rootStep: id => tools.find(t => t.id === id)?.method.type === "apt",
    after: async () => { tools = await api.tools(); overrides.clear(); renderTools(); },
  });
}
$("#go").onclick = openReview;
mgo.onclick = () => goAction();
mcancel.onclick = () => {
  if (running) { api.cancel(); mcancel.disabled = true; mcancel.textContent = "Cancelling…"; return; }
  reviewToken++; modal.hidden = true;
};

// ---- save and load setups ----
const showModal = (title: string) => {
  reviewToken++; $("#mt").textContent = title; $("#mbody").replaceChildren(); logEl.hidden = true; noteEl.hidden = true;
  mgo.hidden = true; mcancel.textContent = "Close"; mcancel.disabled = false; modal.hidden = false;
  return $("#mbody");
};
const textInput = (placeholder: string, label: string) => {
  const i = document.createElement("input"); i.type = "text"; i.placeholder = placeholder; i.setAttribute("aria-label", label); return i;
};

function openExport() {
  const body = showModal("Save setup");
  const order = new Map(tools.map((t, i) => [t.id, i]));
  const ids = [...selection()].sort((a, b) => (order.get(a) ?? 0) - (order.get(b) ?? 0)), rs = [...roles];
  const nameIn = textInput("Name", "Setup name"); nameIn.value = "My setup"; nameIn.maxLength = 80;
  const msg = el("div", "mu"), row = el("div", "btns"), saveBtn = el("button", "bt", "Save to file…"), copyBtn = el("button", "b2", "Copy text");
  const text = () => api.exportSetup(nameIn.value.trim() || "My setup", rs, ids);
  saveBtn.onclick = async () => {
    try {
      let path = await api.pickSavePath(); if (!path) return;
      if (!/\.ya?ml$/i.test(path)) path += ".yaml";
      await api.writeSetupFile(path, await text()); msg.textContent = `Saved to ${path}`;
    } catch (e) { msg.textContent = `Could not save: ${e}`; }
  };
  copyBtn.onclick = async () => {
    try { await navigator.clipboard.writeText(await text()); msg.textContent = "Copied to clipboard."; }
    catch (e) { msg.textContent = `Could not copy: ${e}`; }
  };
  row.append(saveBtn, copyBtn);
  body.append(el("div", "mu", `${ids.length} tools and ${rs.length} role${rs.length === 1 ? "" : "s"} will be saved. Installed tools are included.`), nameIn, row, msg);
}

function applyImport(im: Imported) {
  for (const r of im.roles) roles.add(r);
  for (const id of im.tools) overrides.set(id, true);
  renderRoles(roleList);
  if (im.tools.some(id => { const t = tools.find(x => x.id === id); return t && !visible(t); })) { showAll = true; $<HTMLInputElement>("#showall").checked = true; }
  renderTools();
  const bits = [`Loaded ${im.name ? `"${im.name}"` : "setup"}: ${im.tools.length} tools, ${im.roles.length} roles added to your selection`];
  if (im.unknown_tools.length || im.unknown_roles.length) bits.push(`not in this version of DevSetup: ${[...im.unknown_roles, ...im.unknown_tools].join(", ")}`);
  $("#toast").textContent = bits.join(" · ");
}

function openImport() {
  const body = showModal("Load setup");
  const msg = el("div", "mu"), fileBtn = el("button", "b2", "Open file…");
  const link = textInput("GitHub link or owner/repo", "GitHub link"), fetchBtn = el("button", "b2", "Fetch");
  const ta = document.createElement("textarea"); ta.placeholder = "…or paste setup text here"; ta.setAttribute("aria-label", "Setup text");
  const applyBtn = el("button", "b2", "Apply pasted text");
  const apply = async (text: string) => {
    try { applyImport(await api.parseSetup(text)); modal.hidden = true; } catch (e) { msg.textContent = `${e}`; }
  };
  fileBtn.onclick = async () => {
    try { const p = await api.pickOpenPath(); if (p) await apply(await api.readSetupFile(p)); } catch (e) { msg.textContent = `Could not open: ${e}`; }
  };
  fetchBtn.onclick = async () => {
    msg.textContent = "Fetching…";
    try { await apply(await api.fetchSetup(link.value)); } catch (e) { msg.textContent = `${e}`; }
  };
  applyBtn.onclick = () => apply(ta.value);
  const r1 = el("div", "btns"), r2 = el("div", "btns");
  r1.append(fileBtn); r2.append(link, fetchBtn);
  body.append(el("div", "mu", "Loading adds the setup's tools and roles to your current selection. Nothing is installed until you review the plan."), r1, r2, ta, applyBtn, msg);
}

async function openScan() {
  const body = showModal("Scan this machine"), token = reviewToken;
  body.append(el("div", "mu", "Scanning…"));
  let scan: Scan;
  try { scan = await api.scanMachine(); }
  catch (e) { body.replaceChildren(el("div", "st warn", `! Scan failed: ${e}`)); return; }
  if (token !== reviewToken || modal.hidden) return;
  body.replaceChildren();
  body.append(el("div", "mu", `Found ${scan.found.length} tools from DevSetup's list on ${scan.system}. Scanning only reads; nothing is changed.`));

  const roleBoxes: Array<[string, HTMLInputElement]> = [], toolBoxes: Array<[string, HTMLInputElement]> = [];
  const checkRow = (label: string, tag: string, on: boolean, into: Array<[string, HTMLInputElement]>, id: string) => {
    const row = el("label", "row"), cb = document.createElement("input");
    cb.type = "checkbox"; cb.checked = on; into.push([id, cb]);
    row.append(cb, el("span", "", label), el("span", "tag", tag)); return row;
  };

  const hits = scan.roles.filter(r => r.installed > 0);
  if (hits.length) {
    body.append(el("h3", "", "Roles to include (suggested when 3 or more of a role's tools are installed)"));
    for (const r of hits) body.append(checkRow(r.name, `${r.installed} of ${r.total} tools`, r.suggested, roleBoxes, r.id));
  }

  const det = document.createElement("details"), sum = document.createElement("summary");
  sum.textContent = `Tools to include (${scan.found.length}). Untick any you do not want`; det.append(sum);
  let last = "";
  for (const f of scan.found) {
    if (f.group !== last) { det.append(el("h3", "", f.group)); last = f.group; }
    det.append(checkRow(f.name, f.managed ? `${f.source}, installed by DevSetup` : f.source, true, toolBoxes, f.id));
  }
  body.append(det);

  const ex = scan.extras, notes = [...ex.flatpak.map(x => `flatpak: ${x}`), ...ex.npm.map(x => `npm: ${x}`), ...ex.pipx.map(x => `pipx: ${x}`)];
  if (notes.length) {
    const d2 = document.createElement("details"), s2 = document.createElement("summary");
    s2.textContent = `Also found, not in DevSetup's list (${notes.length}). Saved as comments only`; d2.append(s2);
    for (const n of notes) d2.append(el("div", "mu", n));
    body.append(d2);
  }

  const nameIn = textInput("Name", "Setup name"); nameIn.value = "My machine"; nameIn.maxLength = 80;
  const chosenRoles = () => roleBoxes.filter(([, c]) => c.checked).map(([id]) => id);
  const chosenTools = () => toolBoxes.filter(([, c]) => c.checked).map(([id]) => id);
  const name = () => nameIn.value.trim() || "My machine";
  const msg = el("div", "mu"), row = el("div", "btns");
  const saveBtn = el("button", "bt", "Save to file…"), copyBtn = el("button", "b2", "Copy text"), useBtn = el("button", "b2", "Use as my selection");
  const text = () => api.exportSetup(name(), chosenRoles(), chosenTools(), notes);
  saveBtn.onclick = async () => {
    try {
      let path = await api.pickSavePath(); if (!path) return;
      if (!/\.ya?ml$/i.test(path)) path += ".yaml";
      await api.writeSetupFile(path, await text()); msg.textContent = `Saved to ${path}`;
    } catch (e) { msg.textContent = `Could not save: ${e}`; }
  };
  copyBtn.onclick = async () => {
    try { await navigator.clipboard.writeText(await text()); msg.textContent = "Copied to clipboard."; }
    catch (e) { msg.textContent = `Could not copy: ${e}`; }
  };
  useBtn.onclick = () => { applyImport({ name: name(), roles: chosenRoles(), tools: chosenTools(), unknown_roles: [], unknown_tools: [] }); modal.hidden = true; };
  row.append(saveBtn, copyBtn, useBtn);
  body.append(el("h3", "", "Save this as a setup file"), nameIn, row, msg);
}
$("#scan").onclick = openScan;
$("#save").onclick = openExport;
$("#load").onclick = openImport;

// ---- cleaner ----
let cleanItems: CleanItem[] = [], cleanLoaded = false;
const cleanOn = new Set<string>();
const fmtBytes = (b: number) => b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : b >= 1e6 ? `${(b / 1e6).toFixed(1)} MB` : b >= 1e3 ? `${Math.round(b / 1e3)} kB` : `${b} B`;

async function loadClean() {
  $("#clean-list").replaceChildren(el("div", "mu", "Scanning…")); $<HTMLButtonElement>("#clean-go").disabled = true; $("#clean-sum").textContent = "";
  try { cleanItems = await api.scanClean(); }
  catch (e) { $("#clean-list").replaceChildren(el("div", "st warn", `! Scan failed: ${e}`)); return; }
  cleanOn.clear();
  for (const i of cleanItems) if (i.available && i.risk === "safe" && (i.bytes ?? 0) > 0) cleanOn.add(i.id);
  cleanLoaded = true; renderClean();
}

function renderClean() {
  const box = $("#clean-list"); box.replaceChildren();
  for (const i of cleanItems) {
    const card = el("div", "card"), head = el("label", "row"), cb = document.createElement("input");
    cb.type = "checkbox"; cb.checked = cleanOn.has(i.id); cb.disabled = !i.available;
    cb.onchange = () => { cb.checked ? cleanOn.add(i.id) : cleanOn.delete(i.id); renderClean(); };
    head.append(cb, el("b", "", i.name), el("span", "tag", !i.available ? "unavailable" : i.bytes === null ? "size unknown" : fmtBytes(i.bytes)));
    card.append(head, el("div", "mu", i.description));
    if (i.detail) card.append(el("div", "mu", i.detail));
    if (!i.available && i.reason) card.append(el("div", "st warn", i.reason));
    if (i.risk === "review" && i.available) card.append(el("div", "st warn", "Review first: this can remove things you may still want."));
    const det = document.createElement("details"), sum = document.createElement("summary");
    sum.textContent = "Command"; det.append(sum, el("pre", "", (i.needs_root ? "as root, asks for your password:\n" : "") + i.command));
    card.append(det); box.append(card);
  }
  const chosen = cleanItems.filter(i => cleanOn.has(i.id));
  const total = chosen.reduce((n, i) => n + (i.bytes ?? 0), 0), unknown = chosen.filter(i => i.bytes === null).length;
  $("#clean-sum").textContent = chosen.length ? `About ${fmtBytes(total)} will be freed${unknown ? `, plus ${unknown} item${unknown === 1 ? "" : "s"} with unknown size` : ""} (estimate)` : "Nothing selected";
  const go = $<HTMLButtonElement>("#clean-go"); go.disabled = !chosen.length; go.textContent = chosen.length ? `Clean selected (${chosen.length})` : "Clean selected";
}

function startClean(ids: string[]) {
  return runProgress({
    title: "Cleaning",
    label: id => cleanItems.find(i => i.id === id)?.name ?? id,
    start: () => api.runClean(ids),
    retry: again => { void startClean(again); },
    rootStep: id => !!cleanItems.find(i => i.id === id)?.needs_root,
    after: loadClean,
  });
}

$("#clean-go").onclick = () => { void startClean([...cleanOn]); };
$("#clean-rescan").onclick = () => { void loadClean(); };

// ---- updates ----
let updItems: UpdateItem[] = [], updLoaded = false;
const updOn = new Set<string>();

async function loadUpdates() {
  $("#upd-list").replaceChildren(el("div", "mu", "Checking for updates…")); $<HTMLButtonElement>("#upd-go").disabled = true; $("#upd-sum").textContent = "";
  try { updItems = await api.scanUpdates(); }
  catch (e) { $("#upd-list").replaceChildren(el("div", "st warn", `! Check failed: ${e}`)); return; }
  updOn.clear();
  for (const i of updItems) if (i.available && (i.count === null || i.count > 0)) updOn.add(i.id);
  updLoaded = true; renderUpdates();
}

function renderUpdates() {
  const box = $("#upd-list"); box.replaceChildren();
  for (const i of updItems) {
    const card = el("div", "card"), head = el("label", "row"), cb = document.createElement("input");
    cb.type = "checkbox"; cb.checked = updOn.has(i.id); cb.disabled = !i.available;
    cb.onchange = () => { cb.checked ? updOn.add(i.id) : updOn.delete(i.id); renderUpdates(); };
    const tag = !i.available ? "unavailable" : i.count === null ? "checked when run" : i.count === 0 ? "up to date" : `${i.count} update${i.count === 1 ? "" : "s"}`;
    head.append(cb, el("b", "", i.name), el("span", "tag", tag));
    card.append(head, el("div", "mu", i.description));
    if (i.detail) card.append(el("div", "mu", i.detail));
    if (!i.available && i.reason) card.append(el("div", "st warn", i.reason));
    const det = document.createElement("details"), sum = document.createElement("summary");
    sum.textContent = "Command"; det.append(sum, el("pre", "", (i.needs_root ? "as root, asks for your password:\n" : "") + i.command));
    card.append(det); box.append(card);
  }
  const n = updItems.filter(i => updOn.has(i.id)).length;
  $("#upd-sum").textContent = n ? `${n} selected` : "Nothing selected";
  const go = $<HTMLButtonElement>("#upd-go"); go.disabled = !n; go.textContent = n ? `Update selected (${n})` : "Update selected";
}

function startUpdate(ids: string[]) {
  return runProgress({
    title: "Updating",
    label: id => updItems.find(i => i.id === id)?.name ?? id,
    start: () => api.runUpdates(ids),
    retry: again => { void startUpdate(again); },
    rootStep: id => !!updItems.find(i => i.id === id)?.needs_root,
    after: loadUpdates,
  });
}
$("#upd-go").onclick = () => { void startUpdate([...updOn]); };
$("#upd-rescan").onclick = () => { void loadUpdates(); };

// ---- doctor ----
let docItems: Finding[] = [], docLoaded = false;

async function loadDoctor() {
  $("#doc-list").replaceChildren(el("div", "mu", "Running checks…")); $("#doc-sum").textContent = "";
  try { docItems = await api.scanDoctor(); }
  catch (e) { $("#doc-list").replaceChildren(el("div", "st warn", `! Checks failed: ${e}`)); return; }
  docLoaded = true; renderDoctor();
}

function startFix(fix: FixInfo) {
  return runProgress({
    title: "Fixing", label: () => fix.label,
    start: () => api.runFix(fix.id),
    rootStep: () => fix.kind === "root",
    after: loadDoctor,
  });
}

function fixRow(fix: FixInfo) {
  const row = el("div", "fixrow");
  if (fix.kind === "copy") {
    const btn = el("button", "b2 mini", "Copy");
    btn.onclick = async () => { try { await navigator.clipboard.writeText(fix.command); btn.textContent = "Copied"; } catch { btn.textContent = "Could not copy"; } };
    row.append(el("pre", "", fix.command), btn);
  } else {
    const btn = el("button", "bt", fix.label);
    if (fix.kind === "root") row.append(el("div", "mu", `Runs as root (password prompt): ${fix.command}`)); else row.append(el("div", "mu", `Runs: ${fix.command}`));
    btn.onclick = async () => { if (fix.warning && !(await api.confirm(fix.warning))) return; void startFix(fix); };
    row.append(btn);
  }
  return row;
}

function renderDoctor() {
  const box = $("#doc-list"); box.replaceChildren();
  const mark = { ok: "✓", warn: "!", fail: "✕", info: "i" };
  for (const f of docItems) {
    const card = el("div", "card"), head = el("div", "row");
    head.append(el("span", "st " + f.status, mark[f.status]), el("b", "", f.title)); card.append(head);
    if (f.detail) card.append(el("div", "mu", f.detail));
    if (f.advice) card.append(el("div", "mu", f.advice));
    if (f.fix) card.append(fixRow(f.fix));
    box.append(card);
  }
  const count = (s: string) => docItems.filter(f => f.status === s).length;
  $("#doc-sum").textContent = `${count("fail")} failed, ${count("warn")} warnings, ${count("ok")} ok`;
}
$("#doc-rescan").onclick = () => { void loadDoctor(); };

// ---- git and github ----
let gitState: GitState | null = null, gitLoaded = false;
let gitDraft: { name?: string; email?: string } = {}; // typed but not yet saved, kept across redraws
const gitNotes: Record<string, { text: string; bad: boolean }> = {};
const note = (card: string, text: string, bad = false) => { gitNotes[card] = { text, bad }; };

async function loadGit() {
  if (!gitState) $("#git-list").replaceChildren(el("div", "mu", "Loading…"));
  try { gitState = await api.gitState(); }
  catch (e) { $("#git-list").replaceChildren(el("div", "st warn", `! Could not read Git settings: ${e}`)); return; }
  gitLoaded = true; renderGit();
}

/** Runs an action, remembers its message for the card, and redraws. */
async function gitAct(card: string, work: () => Promise<string | void>) {
  note(card, "Working…");
  renderGit();
  try { const r = await work(); note(card, typeof r === "string" && r ? r : "Done."); }
  catch (e) { note(card, `${e}`, true); }
  await loadGit();
}

function gitCard(id: string, title: string, intro: string) {
  const card = el("div", "card");
  card.append(el("b", "", title), el("div", "mu", intro));
  const n = gitNotes[id]; if (n) card.append(el("div", n.bad ? "st warn" : "mu", n.text));
  return card;
}

function fieldInput(type: "text" | "email" | "password", value: string, label: string) {
  const i = document.createElement("input"); i.type = type; i.value = value; i.placeholder = label; i.setAttribute("aria-label", label); return i;
}

function startGhLogin(protocol: "ssh" | "https") {
  return runProgress({
    title: "Sign in to GitHub",
    label: id => id === "gh-git" ? "Set up git credentials" : "GitHub sign-in",
    start: () => api.ghLogin(protocol),
    after: loadGit,
    onLog: line => {
      const m = /\b[A-Z0-9]{4}-[A-Z0-9]{4}\b/.exec(line); if (!m) return;
      const box = $("#codebox"), open = el("button", "bt", "Open github.com/login/device");
      open.onclick = () => { void api.openUrl("github-device"); };
      box.replaceChildren(el("div", "", "Enter this one-time code on GitHub:"), el("b", "", m[0]), open); box.hidden = false;
    },
  });
}

function renderGit() {
  const s = gitState, box = $("#git-list"); if (!s) return;
  box.replaceChildren();
  if (!s.git_installed) { box.append(el("div", "st warn", "! Git is not installed. Install it from Setup first.")); return; }

  // 1. identity
  const idc = gitCard("id", "1. Your identity", `Git writes this into every commit. Git ${s.git_version.replace("git version ", "")}.`);
  const name = fieldInput("text", gitDraft.name ?? s.name, "Your name"), email = fieldInput("email", gitDraft.email ?? s.email, "you@example.com");
  name.oninput = () => { gitDraft.name = name.value; }; email.oninput = () => { gitDraft.email = email.value; };
  const main = document.createElement("input"); main.type = "checkbox"; main.checked = s.default_branch !== "master";
  const mainRow = el("label", "row"); mainRow.append(main, el("span", "", "Name new repositories' first branch main"));
  const save = el("button", "bt", "Save identity"); save.onclick = () => { void gitAct("id", async () => { await api.gitSaveIdentity(name.value, email.value, main.checked); gitDraft = {}; return "Saved."; }); };
  idc.append(name, email, mainRow, save); box.append(idc);

  // 2. ssh key
  const kc = gitCard("key", "2. SSH key", "Lets you push to GitHub without typing a password. Only the public half is ever shown or sent.");
  if (!s.ssh_installed) kc.append(el("div", "st warn", "! ssh-keygen is not installed (install the OpenSSH client)."));
  else if (s.keys.length) {
    for (const k of s.keys) {
      kc.append(el("div", "mu", `${k.path} ${k.fingerprint}`), el("pre", "", k.public));
      const row = el("div", "btns"), copy = el("button", "b2 mini", "Copy public key");
      copy.onclick = async () => { try { await navigator.clipboard.writeText(k.public); copy.textContent = "Copied"; } catch { copy.textContent = "Could not copy"; } };
      row.append(copy); kc.append(row);
    }
    const row = el("div", "btns"), addPage = el("button", "b2", "Add it on GitHub (browser)"), test = el("button", "b2", "Test connection to GitHub");
    addPage.onclick = () => { void api.openUrl("github-ssh").catch(e => { note("key", `${e}`, true); renderGit(); }); };
    test.onclick = () => { void gitAct("key", async () => { const r = await api.gitTestSsh(); if (!r.ok) throw new Error(r.output || "No answer from GitHub."); return r.output; }); };
    row.append(addPage, test); kc.append(row);
  } else {
    const pass = fieldInput("password", "", "Passphrase (optional)");
    const gen = el("button", "bt", "Create an SSH key");
    kc.append(el("div", "mu", "An ed25519 key is created as ~/.ssh/id_ed25519. A passphrase protects it if the file is ever copied. It is passed to ssh-keygen as an argument, so other users on a shared machine could see it for a moment."), pass, gen);
    gen.onclick = () => { void gitAct("key", () => api.gitGenerateKey(email.value || s.email, pass.value)); };
  }
  box.append(kc);

  // 3. github sign-in
  const gc = gitCard("gh", "3. GitHub sign-in", "Uses the GitHub CLI so you can add keys, clone private repositories and push over HTTPS.");
  if (!s.gh_installed) gc.append(el("div", "st warn", "! GitHub CLI is not installed. Tick it under Git and hosting in Setup."));
  else if (s.gh_user) {
    gc.append(el("div", "st ok", `✓ Signed in as ${s.gh_user}`));
    const row = el("div", "btns");
    for (const k of s.keys.slice(0, 1)) {
      const add = el("button", "b2", "Add my SSH key to GitHub");
      add.onclick = () => { void gitAct("gh", () => api.gitAddKey(k.path, "authentication")); };
      row.append(add);
    }
    const cred = el("button", "b2", "Use my sign-in for HTTPS git"); cred.onclick = () => { void gitAct("gh", () => api.gitSetupCredentials()); };
    row.append(cred); gc.append(row);
  } else {
    const sel = document.createElement("select"); sel.setAttribute("aria-label", "Protocol");
    for (const [v, t] of [["https", "HTTPS (works everywhere)"], ["ssh", "SSH (uses your key)"]]) { const o = document.createElement("option"); o.value = v; o.textContent = t; sel.append(o); }
    const go = el("button", "bt", "Sign in with browser"); go.onclick = () => { void startGhLogin(sel.value as "ssh" | "https"); };
    gc.append(sel, go);
  }
  box.append(gc);

  // 4. signed commits
  const sc = gitCard("sign", "4. Signed commits", "Marks your commits as really yours, using your SSH key (needs Git 2.34 or newer).");
  if (!s.keys.length) sc.append(el("div", "mu", "Create an SSH key first."));
  else {
    sc.append(el("div", s.signing ? "st ok" : "mu", s.signing ? `✓ On, using ${s.signing_key}` : "Off"));
    const sel = document.createElement("select"); sel.setAttribute("aria-label", "Key to sign with");
    for (const k of s.keys) { const o = document.createElement("option"); o.value = k.path; o.textContent = k.path; if (k.path === s.signing_key) o.selected = true; sel.append(o); }
    const row = el("div", "btns"), on = el("button", "bt", s.signing ? "Update" : "Turn on"), off = el("button", "b2", "Turn off");
    on.onclick = () => { void gitAct("sign", async () => { await api.gitSetSigning(true, sel.value, email.value || s.email); return "Signed commits are on."; }); };
    off.onclick = () => { void gitAct("sign", async () => { await api.gitSetSigning(false, "", ""); return "Signed commits are off."; }); };
    row.append(on); if (s.signing) row.append(off); sc.append(sel, row);
    if (s.gh_user) {
      const reg = el("button", "b2", "Also add it to GitHub as a signing key");
      reg.onclick = () => { void gitAct("sign", () => api.gitAddKey(sel.value, "signing")); };
      sc.append(reg, el("div", "mu", "GitHub shows a Verified badge once the key is added as a signing key. If it reports a missing permission, run: gh auth refresh -h github.com -s admin:ssh_signing_key"));
    }
  }
  box.append(sc);
}

// ---- shell setup ----
let shState: ShellState | null = null, shLoaded = false, shTarget: "zsh" | "bash" = "zsh";
let shOpts: ShOpts | null = null;
const shNote: { text: string; bad: boolean } = { text: "", bad: false };
let shPreview = "";

const SH_OPTS: Array<{ key: keyof ShOpts; label: string; tool?: string; avail?: keyof ShellState["available"]; zshOnly?: boolean }> = [
  { key: "history", label: "Better history: bigger, shared between terminals, no duplicates" },
  { key: "completion", label: "Smarter tab completion" },
  { key: "starship", label: "Starship prompt", tool: "starship", avail: "starship" },
  { key: "autosuggestions", label: "Suggestions from your history as you type", tool: "zsh-autosuggestions", avail: "autosuggestions", zshOnly: true },
  { key: "syntax_highlighting", label: "Syntax highlighting while you type", tool: "zsh-syntax-highlighting", avail: "syntax_highlighting", zshOnly: true },
  { key: "zoxide", label: "zoxide: jump to folders with z", tool: "zoxide", avail: "zoxide" },
  { key: "fzf", label: "fzf: Ctrl+R history search, Ctrl+T file search", tool: "fzf", avail: "fzf" },
  { key: "direnv", label: "direnv: per-folder environments", tool: "direnv", avail: "direnv" },
  { key: "mise", label: "mise: activate the version manager", tool: "mise", avail: "mise" },
  { key: "aliases", label: "Handy aliases: ls and ll (eza if installed), .., bat and fd names on Debian" },
  { key: "git_aliases", label: "Git aliases: gs, gd, gl, ga, gc, gco, gpl, gps" },
];

async function loadShell() {
  if (!shState) $("#shell-list").replaceChildren(el("div", "mu", "Loading…"));
  try { shState = await api.shellState(); }
  catch (e) { $("#shell-list").replaceChildren(el("div", "st warn", `! Could not read shell settings: ${e}`)); return; }
  if (!shOpts) {
    const a = shState.available;
    shOpts = { history: true, completion: true, aliases: true, git_aliases: false, starship: a.starship, autosuggestions: a.autosuggestions, syntax_highlighting: a.syntax_highlighting, zoxide: a.zoxide, fzf: a.fzf, direnv: a.direnv, mise: a.mise };
    if (shState.login_shell.endsWith("bash") && !shState.zsh_installed) shTarget = "bash";
  }
  shLoaded = true; renderShell();
}

/** Ticks tools in Setup and switches there, so they can be installed with the usual review. */
function tickInSetup(ids: string[]) {
  applyImport({ name: "", roles: [], tools: ids, unknown_roles: [], unknown_tools: [] });
  $("#toast").textContent = `Ticked ${ids.map(name).join(", ")}. Review the plan to install ${ids.length === 1 ? "it" : "them"}.`;
  showView("setup");
}

function renderShell() {
  const s = shState, o = shOpts, box = $("#shell-list"); if (!s || !o) return;
  box.replaceChildren();

  // 1. login shell
  const lc = el("div", "card");
  lc.append(el("b", "", "1. Login shell"), el("div", "mu", `Your login shell is ${s.login_shell || "unknown"}. Terminals start it by default.`));
  if (!s.zsh_installed) {
    const b = el("button", "b2", "Tick zsh in Setup"); b.onclick = () => tickInSetup(["zsh"]);
    lc.append(el("div", "st warn", "! zsh is not installed."), b);
  } else if (!s.login_shell.endsWith("/zsh")) {
    const b = el("button", "b2", "Make zsh my login shell");
    b.onclick = () => { void runProgress({ title: "Changing login shell", label: () => "Login shell", start: () => api.setLoginShell("zsh"), rootStep: () => true, after: loadShell }); };
    lc.append(b, el("div", "mu", "Asks for your password. Log out and back in afterwards."));
  } else lc.append(el("div", "st ok", "✓ zsh is your login shell"));
  box.append(lc);

  // 2. configuration
  const cc = el("div", "card");
  cc.append(el("b", "", "2. What to add"));
  const sel = document.createElement("select"); sel.setAttribute("aria-label", "Shell to configure");
  for (const v of ["zsh", "bash"] as const) { const op = document.createElement("option"); op.value = v; op.textContent = `${v} (${v === "zsh" ? "~/.zshrc" : "~/.bashrc"})`; op.selected = v === shTarget; sel.append(op); }
  sel.onchange = () => { shTarget = sel.value as "zsh" | "bash"; shPreview = ""; shNote.text = ""; renderShell(); };
  cc.append(sel);
  const missing: string[] = [];
  for (const d of SH_OPTS) {
    if (d.zshOnly && shTarget !== "zsh") continue;
    const row = el("label", "row"), cb = document.createElement("input");
    cb.type = "checkbox"; cb.checked = o[d.key];
    cb.onchange = () => { o[d.key] = cb.checked; shPreview = ""; renderShell(); };
    const lack = !!d.avail && !s.available[d.avail];
    row.append(cb, el("span", "", d.label), el("span", "tag", lack ? "not installed" : ""));
    if (lack && o[d.key] && d.tool) missing.push(d.tool);
    cc.append(row);
  }
  const rc = s.files.find(f => f.shell === shTarget);
  if (rc?.damaged) cc.append(el("div", "st warn", `! ${rc.damaged}`));
  const btns = el("div", "btns"), prev = el("button", "b2", "Preview"), apply = el("button", "bt", rc?.has_block ? "Update" : "Apply"), rem = el("button", "b2", "Remove");
  const run = (work: () => Promise<string | void>) => async () => {
    shNote.text = "Working…"; shNote.bad = false; renderShell();
    try { const r = await work(); shNote.text = typeof r === "string" && r ? r : "Done."; shNote.bad = false; }
    catch (e) { shNote.text = `${e}`; shNote.bad = true; }
    shPreview = ""; shState = await api.shellState(); renderShell();
  };
  prev.onclick = async () => {
    try {
      const p = await api.shellPreview(shTarget, o);
      shPreview = `${p.exists ? (p.had_block ? "Replaces the DevSetup block in" : "Adds this block to the end of") : "Creates"} ${p.path}:\n\n${p.block}` + (p.warnings.length ? `\n\nHeads-up:\n- ${p.warnings.join("\n- ")}` : "");
      shNote.text = ""; renderShell();
    } catch (e) { shNote.text = `${e}`; shNote.bad = true; renderShell(); }
  };
  apply.onclick = run(() => api.shellApply(shTarget, o));
  rem.onclick = run(() => api.shellRemove(shTarget));
  btns.append(prev, apply); if (rc?.has_block) btns.append(rem);
  cc.append(btns);
  if (shNote.text) cc.append(el("div", shNote.bad ? "st warn" : "mu", shNote.text));
  if (shPreview) cc.append(el("pre", "", shPreview));
  box.append(cc);

  // 3. missing tools
  if (missing.length) {
    const mc = el("div", "card"), b = el("button", "b2", "Tick them in Setup");
    b.onclick = () => tickInSetup(missing);
    mc.append(el("b", "", "3. Install what is missing"), el("div", "mu", `Needed for the options you ticked: ${missing.map(name).join(", ")}. The lines are safe without them (they only act when the tool exists), but nothing happens until you install them.`), b);
    box.append(mc);
  }
}

// ---- vs code extension packs ----
let vsState: VsState | null = null, vsLoaded = false, vsEditor = "";
const vsOn = new Set<string>(); // lower-case extension ids

const vsName = (id: string) => vsState?.packs.flatMap(p => p.extensions).find(x => x.id.toLowerCase() === id.toLowerCase())?.name ?? id;
const vsHave = () => new Set((vsState?.editors.find(e => e.id === vsEditor)?.installed ?? []).map(x => x.toLowerCase()));

async function loadVs() {
  if (!vsState) $("#vs-list").replaceChildren(el("div", "mu", "Looking for VS Code…"));
  try { vsState = await api.vscodeState(); }
  catch (e) { $("#vs-list").replaceChildren(el("div", "st warn", `! Could not look for VS Code: ${e}`)); return; }
  if (!vsState.editors.some(e => e.id === vsEditor)) vsEditor = vsState.editors[0]?.id ?? "";
  const have = vsHave();
  vsOn.clear();
  for (const p of vsState.packs) {
    if (!p.roles.includes("all") && !p.roles.some(r => roles.has(r))) continue;
    for (const x of p.extensions) if (!have.has(x.id.toLowerCase())) vsOn.add(x.id.toLowerCase());
  }
  vsLoaded = true; renderVs();
}

function renderVs() {
  const s = vsState, box = $("#vs-list"); if (!s) return;
  box.replaceChildren();
  const ec = el("div", "card"); ec.append(el("b", "", "Editor"));
  if (!s.editors.length) {
    const b = el("button", "b2", "Tick VS Code in Setup"); b.onclick = () => tickInSetup(["vscode"]);
    ec.append(el("div", "st warn", "! No VS Code or VSCodium found."), b);
  } else {
    const sel = document.createElement("select"); sel.setAttribute("aria-label", "Editor");
    for (const e of s.editors) { const o = document.createElement("option"); o.value = e.id; o.textContent = e.name; o.selected = e.id === vsEditor; sel.append(o); }
    sel.onchange = () => { vsEditor = sel.value; void loadVs(); };
    ec.append(sel);
  }
  box.append(ec);

  const have = vsHave(), usable = s.editors.length > 0;
  for (const p of s.packs) {
    const card = el("div", "card"), head = el("label", "row"), all = document.createElement("input");
    const todo = p.extensions.filter(x => !have.has(x.id.toLowerCase()));
    all.type = "checkbox"; all.disabled = !usable || !todo.length;
    all.checked = todo.length > 0 && todo.every(x => vsOn.has(x.id.toLowerCase()));
    all.onchange = () => { for (const x of todo) { if (all.checked) vsOn.add(x.id.toLowerCase()); else vsOn.delete(x.id.toLowerCase()); } renderVs(); };
    head.append(all, el("b", "", p.name), el("span", "tag", `${p.extensions.length - todo.length} of ${p.extensions.length} installed`));
    card.append(head, el("div", "mu", p.description));
    for (const x of p.extensions) {
      const row = el("label", "row"), cb = document.createElement("input"), done = have.has(x.id.toLowerCase());
      cb.type = "checkbox"; cb.disabled = done || !usable; cb.checked = done || vsOn.has(x.id.toLowerCase());
      cb.onchange = () => { if (cb.checked) vsOn.add(x.id.toLowerCase()); else vsOn.delete(x.id.toLowerCase()); renderVs(); };
      row.append(cb, el("span", "", x.name), el("span", "tag", done ? "installed" : x.id)); card.append(row);
    }
    box.append(card);
  }
  const n = [...vsOn].filter(id => !have.has(id)).length;
  $("#vs-sum").textContent = n ? `${n} selected` : "Nothing selected";
  const go = $<HTMLButtonElement>("#vs-go"); go.disabled = !n || !usable; go.textContent = n ? `Install selected (${n})` : "Install selected";
}

function startVs(ids: string[]) {
  return runProgress({
    title: "Installing VS Code extensions",
    label: vsName,
    start: () => api.vscodeInstall(vsEditor, ids),
    retry: again => { void startVs(again); },
    after: loadVs,
  });
}
$("#vs-go").onclick = () => {
  const have = vsHave();
  const all = vsState?.packs.flatMap(p => p.extensions) ?? [];
  const ids = [...vsOn].filter(id => !have.has(id)).map(id => all.find(x => x.id.toLowerCase() === id)?.id ?? id);
  void startVs(ids);
};

// ---- drivers (read-only) ----
let drvReport: DriverReport | null = null, drvLoaded = false;

async function loadDrivers() {
  $("#drv-list").replaceChildren(el("div", "mu", "Checking your hardware…")); $("#drv-sum").textContent = "";
  try { drvReport = await api.scanDrivers(); }
  catch (e) { $("#drv-list").replaceChildren(el("div", "st warn", `! Could not read your hardware: ${e}`)); return; }
  drvLoaded = true; renderDrivers();
}

function renderDrivers() {
  const r = drvReport, box = $("#drv-list"); if (!r) return;
  box.replaceChildren();
  const hw = el("div", "card"); hw.append(el("b", "", "Your hardware"));
  if (!r.gpus.length) hw.append(el("div", "mu", "No graphics device found."));
  for (const g of r.gpus) hw.append(el("div", "mu", `${g.vendor}: ${g.name} (${g.id}), driver: ${g.driver || "none"}`));
  hw.append(el("div", "mu", `System: ${r.distro || "unknown"}. Secure Boot: ${r.secure_boot}.`));
  box.append(hw);
  const mark = { ok: "✓", info: "i", warn: "!" };
  for (const a of r.advice) {
    const card = el("div", "card"), head = el("div", "row");
    head.append(el("span", "st " + a.level, mark[a.level]), el("b", "", a.title)); card.append(head);
    if (a.detail) card.append(el("div", "mu", a.detail));
    a.steps.forEach((s, i) => card.append(el("div", "mu", `${i + 1}. ${s}`)));
    if (a.commands.length) {
      const text = a.commands.join("\n"), copy = el("button", "b2 mini", "Copy commands");
      copy.onclick = async () => { try { await navigator.clipboard.writeText(text); copy.textContent = "Copied"; } catch { copy.textContent = "Could not copy"; } };
      const wrap = el("div", "fixrow"); wrap.append(el("pre", "", text), copy); card.append(wrap);
    }
    box.append(card);
  }
  const warns = r.advice.filter(a => a.level === "warn").length;
  $("#drv-sum").textContent = warns ? `${warns} thing${warns === 1 ? "" : "s"} to look at` : "Nothing needs attention";
}
$("#drv-rescan").onclick = () => { void loadDrivers(); };

// ---- stacks ----
let stState: StacksState | null = null, stLoaded = false, stName = "myapp";
const stSel = new Set<string>(), stGui = new Set<string>();
const stNote = { text: "", bad: false };

async function loadStacks() {
  if (!stState) $("#st-list").replaceChildren(el("div", "mu", "Looking for Docker…"));
  try { stState = await api.stacksState(); }
  catch (e) { $("#st-list").replaceChildren(el("div", "st warn", `! Could not read stacks: ${e}`)); return; }
  stLoaded = true; renderStacks();
}

function stackRun(title: string, start: () => Promise<void>) {
  return runProgress({ title, label: () => "Stack", start, after: loadStacks });
}

function copyButton(text: string, label = "Copy") {
  const b = el("button", "b2 mini", label);
  b.onclick = async () => { try { await navigator.clipboard.writeText(text); b.textContent = "Copied"; } catch { b.textContent = "Could not copy"; } };
  return b;
}

function renderStacks() {
  const s = stState, box = $("#st-list"); if (!s) return;
  box.replaceChildren();
  const ready = s.docker.daemon && s.docker.compose;

  // new stack
  const nc = el("div", "card"); nc.append(el("b", "", "New stack"));
  if (!s.docker.daemon) nc.append(el("div", "st warn", `! Docker is not available: ${s.docker.reason}. Docker must be installed and running, and you must be in the docker group. Doctor can check both.`));
  else if (!s.docker.compose) {
    const b = el("button", "b2", "Tick Docker Compose in Setup"); b.onclick = () => tickInSetup(["docker-compose"]);
    nc.append(el("div", "st warn", "! Docker Compose is not installed."), b);
  }
  const nameIn = document.createElement("input"); nameIn.type = "text"; nameIn.value = stName; nameIn.maxLength = 30; nameIn.setAttribute("aria-label", "Stack name"); nameIn.placeholder = "Stack name";
  nameIn.oninput = () => { stName = nameIn.value; updateCreate(); };
  nc.append(nameIn, el("div", "mu", "Start from a combination, or pick services:"));
  const combos = el("div", "btns");
  for (const c of s.catalog.combos) {
    const b = el("button", "b2", c.name); b.title = c.description;
    b.onclick = () => { stSel.clear(); stGui.clear(); c.services.forEach(x => stSel.add(x)); c.guis.forEach(x => stGui.add(x)); renderStacks(); };
    combos.append(b);
  }
  nc.append(combos);
  for (const d of s.catalog.services) {
    const row = el("label", "row"), cb = document.createElement("input");
    cb.type = "checkbox"; cb.checked = stSel.has(d.id);
    cb.onchange = () => { if (cb.checked) stSel.add(d.id); else { stSel.delete(d.id); stGui.delete(d.id); } renderStacks(); };
    row.append(cb, el("span", "", `${d.name}: ${d.description}`), el("span", "tag", `port ${d.host_port}`)); nc.append(row);
    if (d.gui && stSel.has(d.id)) {
      const g = el("label", "row"), gc = document.createElement("input"); gc.type = "checkbox"; gc.checked = stGui.has(d.id);
      gc.onchange = () => { if (gc.checked) stGui.add(d.id); else stGui.delete(d.id); };
      g.style.marginLeft = "26px"; g.append(gc, el("span", "", `with ${d.gui.name}`)); nc.append(g);
    }
  }
  const create = el("button", "bt", "Create and start") as HTMLButtonElement;
  const updateCreate = () => { create.disabled = !ready || !stSel.size || !/^[a-z0-9]([a-z0-9-]{0,28}[a-z0-9])?$/.test(stName); };
  create.onclick = () => { stNote.text = ""; void stackRun("Creating stack", () => api.stackCreate(stName, [...stSel], [...stGui].filter(x => stSel.has(x)))); };
  updateCreate();
  nc.append(create, el("div", "mu", "If a service's usual port is busy or used by another stack, the next free port is used and shown below. The first start downloads the Docker images."));
  if (stNote.text) nc.append(el("div", stNote.bad ? "st warn" : "mu", stNote.text));
  box.append(nc);

  // existing stacks
  for (const st of s.stacks) {
    const card = el("div", "card"), head = el("div", "row");
    head.append(el("b", "", st.name), el("span", "tag", `${st.running} of ${st.containers.length} running`)); card.append(head);
    if (st.created) card.append(el("div", "mu", `Created ${new Date(st.created * 1000).toLocaleDateString()}`));
    for (const c of st.connections) {
      const w = el("div", "fixrow"); w.append(el("div", "mu", c.service), el("pre", "", c.value), copyButton(c.value)); card.append(w);
    }
    if (st.links.length) {
      const row = el("div", "btns");
      st.links.forEach((l, i) => { const b = el("button", "b2", `Open ${l.label}`); b.onclick = () => { void api.stackOpen(st.name, i).catch(e => { stNote.text = `${e}`; stNote.bad = true; renderStacks(); }); }; row.append(b); });
      card.append(row);
    }
    for (const n of st.notes) card.append(el("div", "mu", n));
    const row = el("div", "btns");
    if (st.running < st.containers.length) { const b = el("button", "bt", "Start"); b.onclick = () => { void stackRun("Starting stack", () => api.stackAction(st.name, "start")); }; row.append(b); }
    if (st.running > 0) { const b = el("button", "b2", "Stop"); b.onclick = () => { void stackRun("Stopping stack", () => api.stackAction(st.name, "stop")); }; row.append(b); }
    if (st.env_text) row.append(copyButton(st.env_text, "Copy .env lines"));
    const del = el("button", "b2", "Destroy…");
    del.onclick = async () => {
      if (!(await api.confirm(`Destroy ${st.name}? This deletes its containers, its network and its volumes, including all data stored in them. It cannot be undone.`))) return;
      void stackRun("Destroying stack", () => api.stackAction(st.name, "destroy"));
    };
    row.append(del); card.append(row); box.append(card);
  }
  $("#st-sum").textContent = s.stacks.length ? `${s.stacks.length} stack${s.stacks.length === 1 ? "" : "s"}` : "No stacks yet";
}
$("#st-rescan").onclick = () => { void loadStacks(); };

type View = "setup" | "clean" | "updates" | "doctor" | "git" | "shell" | "vscode" | "drivers" | "stacks";
function showView(v: View) {
  $("#v-setup").hidden = v !== "setup"; $("#v-clean").hidden = v !== "clean"; $("#v-updates").hidden = v !== "updates"; $("#v-doctor").hidden = v !== "doctor"; $("#v-git").hidden = v !== "git"; $("#v-shell").hidden = v !== "shell"; $("#v-vscode").hidden = v !== "vscode"; $("#v-drivers").hidden = v !== "drivers"; $("#v-stacks").hidden = v !== "stacks";
  document.querySelectorAll<HTMLButtonElement>("#nav button").forEach(b => {
    if (b.dataset.v === v) b.setAttribute("aria-current", "page"); else b.removeAttribute("aria-current");
  });
  if (v === "clean" && !cleanLoaded) void loadClean();
  if (v === "updates" && !updLoaded) void loadUpdates();
  if (v === "doctor" && !docLoaded) void loadDoctor();
  if (v === "git" && !gitLoaded) void loadGit();
  if (v === "shell" && !shLoaded) void loadShell();
  if (v === "vscode" && !vsLoaded) void loadVs();
  if (v === "drivers" && !drvLoaded) void loadDrivers();
  if (v === "stacks" && !stLoaded) void loadStacks();
}
document.querySelectorAll<HTMLButtonElement>("#nav button").forEach(b => { b.onclick = () => showView(b.dataset.v as View); });


initTheme();
(async () => {
  const [sys, m, t] = await Promise.all([api.system(), api.manifest(), api.tools()]);
  $("#sys").textContent = sys.name; tools = t;
  roleList = m.roles; renderRoles(roleList); renderTools();
})();

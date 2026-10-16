import "./style.css";
import { api, Tool, Role, Plan, Imported, Scan, CleanItem } from "./api";

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
};

/** Shows live progress for a run (install or clean): steps, log, cancel, and retry of failed steps. */
async function runProgress(o: RunOptions) {
  const body = $("#mbody"), box = el("div"), rows = new Map<string, HTMLElement>(), states = new Map<string, string>();
  reviewToken++; modal.hidden = false;
  body.replaceChildren(box); logEl.textContent = ""; logEl.hidden = false; noteEl.hidden = true;
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
    api.onLog(l => { logEl.textContent += `[${l.id}] ${l.line}\n`; logEl.scrollTop = logEl.scrollHeight; }),
    api.onDone(async d => {
      stopListening(); running = false; noteEl.hidden = true;
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

function showView(v: "setup" | "clean") {
  $("#v-setup").hidden = v !== "setup"; $("#v-clean").hidden = v !== "clean";
  document.querySelectorAll<HTMLButtonElement>("#nav button").forEach(b => {
    if (b.dataset.v === v) b.setAttribute("aria-current", "page"); else b.removeAttribute("aria-current");
  });
  if (v === "clean" && !cleanLoaded) void loadClean();
}
document.querySelectorAll<HTMLButtonElement>("#nav button").forEach(b => { b.onclick = () => showView(b.dataset.v as "setup" | "clean"); });


initTheme();
(async () => {
  const [sys, m, t] = await Promise.all([api.system(), api.manifest(), api.tools()]);
  $("#sys").textContent = sys.name; tools = t;
  roleList = m.roles; renderRoles(roleList); renderTools();
})();

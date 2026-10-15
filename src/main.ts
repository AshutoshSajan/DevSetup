import "./style.css";
import { api, Tool, Role, Plan } from "./api";

const $ = <T extends HTMLElement = HTMLElement>(s: string) => document.querySelector(s) as T;
const el = (tag: string, cls = "", text = "") => { const e = document.createElement(tag); e.className = cls; e.textContent = text; return e; };

let tools: Tool[] = [];
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
    cb.type = "checkbox";
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

async function startRun(install: string[], remove: string[]) {
  const body = $("#mbody"), box = el("div"), rows = new Map<string, HTMLElement>(), states = new Map<string, string>();
  body.replaceChildren(box); logEl.textContent = ""; logEl.hidden = false; noteEl.hidden = true;
  running = true;
  $("#mt").textContent = "Installing"; mgo.hidden = true; mcancel.textContent = "Cancel"; mcancel.disabled = false;
  const row = (id: string) => {
    let r = rows.get(id);
    if (!r) { r = el("div", "step"); r.append(el("span", "", name(id)), el("span", "st", "")); box.append(r); rows.set(id, r); }
    return r;
  };
  unlisten = await Promise.all([
    api.onStep(s => {
      const st = row(s.id).lastElementChild as HTMLElement; states.set(s.id, s.state);
      st.className = "st " + s.state; st.textContent = s.message ? `${s.state}: ${s.message}` : s.state;
      noteEl.hidden = !(s.state === "running" && tools.find(t => t.id === s.id)?.method.type === "apt");
    }),
    api.onLog(l => { logEl.textContent += `[${l.id}] ${l.line}\n`; logEl.scrollTop = logEl.scrollHeight; }),
    api.onDone(async d => {
      stopListening(); running = false; noteEl.hidden = true;
      $("#mt").textContent = d.cancelled ? "Cancelled" : d.failed ? `Finished, ${d.failed} failed` : "Finished";
      mcancel.textContent = "Close"; mcancel.disabled = false;
      const unfinished = [...states].filter(([, st]) => st === "failed" || st === "cancelled").map(([id]) => id);
      if (unfinished.length) {
        const retry = (ids: string[]) => startRun(ids.filter(i => !remove.includes(i)), ids.filter(i => remove.includes(i)));
        goAction = () => retry(unfinished);
        mgo.textContent = `Retry unfinished (${unfinished.length})`; mgo.hidden = false; mgo.disabled = false;
        for (const id of unfinished) {
          const b = el("button", "b2 mini", "Retry"); b.onclick = () => retry([id]); rows.get(id)?.append(b);
        }
      }
      tools = await api.tools(); overrides.clear(); renderTools();
    }),
  ]);
  try { await api.run(install, remove, strict); }
  catch (e) { stopListening(); running = false; logEl.textContent += `error: ${e}\n`; $("#mt").textContent = "Could not start"; mcancel.textContent = "Close"; mcancel.disabled = false; }
}
$("#go").onclick = openReview;
mgo.onclick = () => goAction();
mcancel.onclick = () => {
  if (running) { api.cancel(); mcancel.disabled = true; mcancel.textContent = "Cancelling…"; return; }
  reviewToken++; modal.hidden = true;
};

initTheme();
(async () => {
  const [sys, m, t] = await Promise.all([api.system(), api.manifest(), api.tools()]);
  $("#sys").textContent = sys.name; tools = t;
  renderRoles(m.roles); renderTools();
})();

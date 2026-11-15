const T = window.__TAURI__;
const invoke = T && T.core ? T.core.invoke : async () => { throw new Error("Run this inside the Tauri app"); };
const listen = T && T.event ? T.event.listen : async () => () => {};

const $ = (s, r = document) => r.querySelector(s);
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const GB = 1024 ** 3, MB = 1024 ** 2, KB = 1024;
function fmt(b) {
  if (b >= GB) return (b / GB).toFixed(b >= 10 * GB ? 0 : 1) + " GB";
  if (b >= MB) return Math.round(b / MB) + " MB";
  if (b >= KB) return Math.round(b / KB) + " KB";
  return b + " B";
}
const hue = (s) => [...s].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7);
const store = {
  get(k, d) { try { return localStorage.getItem(k) ?? d; } catch (_) { return d; } },
  set(k, v) { try { localStorage.setItem(k, v); } catch (_) {} },
};

let apps = [];
let sortBy = "size";
let loaded = { apps: false, storage: false, cleanup: false, files: false };
let caches = [];
let drives = [];
let curDrive = "C";
let userDir = "C:\\";
let known = [];

function toast(msg) {
  const t = $("#toast");
  t.textContent = msg;
  t.classList.remove("hidden");
  clearTimeout(toast._t);
  toast._t = setTimeout(() => t.classList.add("hidden"), 5000);
}
function modal(html) { $("#modal-box").innerHTML = html; $("#modal").classList.remove("hidden"); }
function closeModal() { $("#modal").classList.add("hidden"); }
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !$("#modal").classList.contains("hidden") && ($("#m-cancel") || $("#m-ok"))) closeModal();
});

// ---------- theme ----------
const themes = ["system", "light", "dark"];
function applyTheme(t) {
  if (t === "system") document.documentElement.removeAttribute("data-theme");
  else document.documentElement.setAttribute("data-theme", t);
  $("#theme-btn").textContent = "Theme: " + t;
  store.set("theme", t);
}
$("#theme-btn").addEventListener("click", () => {
  const cur = store.get("theme", "system");
  applyTheme(themes[(themes.indexOf(cur) + 1) % themes.length]);
});
applyTheme(store.get("theme", "system"));

// ---------- navigation ----------
function show(page) {
  document.querySelectorAll(".page").forEach((p) => p.classList.add("hidden"));
  $("#page-" + page).classList.remove("hidden");
  document.querySelectorAll(".nav").forEach((n) => n.classList.toggle("on", n.dataset.page === page));
  if (page === "apps" && !loaded.apps) loadApps();
  if (page === "storage" && !loaded.storage) { loaded.storage = true; go(userDir); }
  if (page === "cleanup" && !loaded.cleanup) loadCaches();
  if (page === "files" && !loaded.files) { loaded.files = true; setupFiles(); }
}
document.querySelectorAll(".nav").forEach((n) => n.addEventListener("click", () => show(n.dataset.page)));
$("#go-cleanup").addEventListener("click", () => show("cleanup"));
$("#view-apps").addEventListener("click", () => show("apps"));

// ---------- dashboard and drives ----------
async function loadDrives() {
  try {
    drives = await invoke("list_drives");
    if (!drives.find((d) => d.letter === curDrive) && drives.length) curDrive = drives[0].letter;
    renderDrives();
    $("#st-drive").innerHTML = drives.map((d) => `<option value="${d.letter}">${d.letter}:</option>`).join("");
    updatePathBar();
  } catch (e) { toast(String(e)); }
}
function renderDrives() {
  $("#drive-chips").innerHTML = drives.length > 1
    ? drives.map((d) => `<button class="chip ${d.letter === curDrive ? "on" : ""}" data-drive="${d.letter}">${d.letter}:${d.removable ? " (removable)" : ""}</button>`).join("")
    : "";
  const d = drives.find((x) => x.letter === curDrive);
  if (!d) return;
  const used = d.total - d.free;
  const pct = Math.round((used / d.total) * 100);
  const circ = 2 * Math.PI * 44;
  $("#ring").setAttribute("stroke-dasharray", `${(circ * pct) / 100} ${circ}`);
  $("#ring").setAttribute("class", "ring-fg" + (pct >= 90 ? " crit" : pct >= 80 ? " warn" : ""));
  $("#ring-text").textContent = pct + "%";
  $("#drive-label").textContent = d.letter + ": drive";
  $("#free-text").textContent = fmt(d.free) + " free";
  $("#total-text").textContent = "of " + fmt(d.total);
}
$("#drive-chips").addEventListener("click", (e) => {
  const b = e.target.closest("[data-drive]");
  if (b) { curDrive = b.dataset.drive; renderDrives(); }
});
$("#refresh-dash").addEventListener("click", () => { loadDrives(); loaded.apps = false; loadApps(); loadCaches(); });

// ---------- apps ----------
const iconCache = new Map();
const iconQueue = [];
let iconBusy = 0;
const ikey = (a) => a.name + "|" + a.version;
function applyIcon(key, uri) {
  document.querySelectorAll(".avatar[data-ik]").forEach((el) => {
    if (el.dataset.ik === key && !el.classList.contains("has-icon")) {
      el.innerHTML = `<img src="${uri}" alt="" />`;
      el.classList.add("has-icon");
    }
  });
}
function reapplyIcons() {
  document.querySelectorAll(".avatar[data-ik]").forEach((el) => {
    const u = iconCache.get(el.dataset.ik);
    if (u) applyIcon(el.dataset.ik, u);
  });
}
function pumpIcons() {
  while (iconBusy < 4 && iconQueue.length) {
    const a = iconQueue.shift();
    iconBusy++;
    invoke("get_app_icon", { name: a.name, version: a.version })
      .then((uri) => { if (uri) { iconCache.set(ikey(a), uri); applyIcon(ikey(a), uri); } })
      .catch(() => {})
      .finally(() => { iconBusy--; pumpIcons(); });
  }
}
function queueIcons(list) {
  for (const a of list) {
    const k = ikey(a);
    if (iconCache.has(k)) continue;
    iconCache.set(k, null);
    iconQueue.push(a);
  }
  pumpIcons();
}

function appRow(a, i, withButton) {
  const meta = [a.publisher, a.version].filter(Boolean).map(esc).join(" · ");
  return `<div class="row"><div class="avatar" data-ik="${esc(ikey(a))}" style="--h:${hue(a.name)}">${esc((a.name[0] || "?").toUpperCase())}</div>
    <div class="grow"><div class="title">${esc(a.name)}</div><div class="small muted title">${meta}</div></div>
    <div class="size">${a.size ? fmt(a.size) : "—"}</div>
    ${withButton ? `<button class="btn" data-uninstall="${i}">Uninstall + clean</button>` : ""}</div>`;
}
async function loadApps() {
  loaded.apps = true;
  try {
    apps = await invoke("list_apps");
    renderApps();
    $("#dash-apps").className = "";
    $("#dash-apps").innerHTML = apps.slice(0, 5).map((a, i) => appRow(a, i, false)).join("");
    reapplyIcons();
    queueIcons([...apps.slice(0, 5), ...apps]);
  } catch (e) { loaded.apps = false; $("#apps-list").textContent = String(e); }
}
function renderApps() {
  const q = $("#app-search").value.trim().toLowerCase();
  const rows = apps.map((a, i) => ({ a, i })).filter(({ a }) => !q || a.name.toLowerCase().includes(q) || a.publisher.toLowerCase().includes(q));
  if (sortBy === "name") rows.sort((x, y) => x.a.name.localeCompare(y.a.name));
  $("#app-count").textContent = `${rows.length} app${rows.length === 1 ? "" : "s"}` + (q ? " match" : " installed");
  $("#apps-list").className = "";
  $("#apps-list").innerHTML = rows.length ? rows.map(({ a, i }) => appRow(a, i, true)).join("") : '<div class="muted pad">No matching apps</div>';
  reapplyIcons();
}
$("#app-search").addEventListener("input", renderApps);
$("#app-sort").addEventListener("change", (e) => { sortBy = e.target.value; renderApps(); });
$("#refresh-apps").addEventListener("click", () => { loaded.apps = false; $("#apps-list").className = "muted pad"; $("#apps-list").innerHTML = '<span class="spin"></span>Loading…'; loadApps(); });
$("#apps-list").addEventListener("click", (e) => {
  const b = e.target.closest("[data-uninstall]");
  if (b) confirmUninstall(apps[Number(b.dataset.uninstall)]);
});

function confirmUninstall(app) {
  modal(`<h2>Uninstall ${esc(app.name)}?</h2>
    <p class="muted">This runs the app's own uninstaller first. Afterwards you can review leftover folders and choose what to remove. Nothing else is deleted.</p>
    <div class="btns"><button class="btn" id="m-cancel">Cancel</button><button class="btn danger" id="m-go">Uninstall</button></div>`);
  $("#m-cancel").onclick = closeModal;
  $("#m-go").onclick = async () => {
    modal(`<h2>Uninstalling…</h2><p class="muted"><span class="spin"></span>Finish the uninstaller window if one appears.</p>`);
    try { await invoke("uninstall_app", { name: app.name, version: app.version }); }
    catch (e) { closeModal(); toast(String(e)); return; }
    modal(`<h2>Looking for leftovers…</h2><p class="muted"><span class="spin"></span>Scanning app data folders.</p>`);
    try {
      const left = await invoke("find_leftovers", { name: app.name, publisher: app.publisher, installLocation: app.install_location });
      loaded.apps = false; loadApps(); loadDrives();
      reviewLeftovers(app, left);
    } catch (e) { closeModal(); toast(String(e)); loaded.apps = false; loadApps(); }
  };
}
function reviewLeftovers(app, left) {
  if (!left.length) {
    modal(`<h2>${esc(app.name)} removed</h2><p class="muted">No leftover folders were found.</p><div class="btns"><button class="btn primary" id="m-ok">Done</button></div>`);
    $("#m-ok").onclick = closeModal;
    return;
  }
  const total = left.reduce((s, l) => s + l.size, 0);
  modal(`<h2>Leftover data for ${esc(app.name)}</h2>
    <p class="muted">Found ${fmt(total)} in ${left.length} folder${left.length > 1 ? "s" : ""}. Untick anything you want to keep.</p>
    <div id="lo-list">${left.map((l, i) => `<label class="chk"><input type="checkbox" checked data-i="${i}" /><div class="grow"><div class="p">${esc(l.path)}</div><div class="small muted">${fmt(l.size)}</div></div></label>`).join("")}</div>
    <label class="opt"><input type="checkbox" id="lo-perm" /> Delete permanently (otherwise moved to the Recycle Bin)</label>
    <div class="btns"><button class="btn" id="m-cancel">Keep all</button><button class="btn danger" id="m-del">Remove selected</button></div>`);
  $("#m-cancel").onclick = closeModal;
  $("#m-del").onclick = async () => {
    const paths = [...document.querySelectorAll("#lo-list input:checked")].map((c) => left[Number(c.dataset.i)].path);
    if (!paths.length) { closeModal(); return; }
    try {
      const res = await invoke("delete_paths", { paths, permanent: $("#lo-perm").checked });
      const ok = res.filter((r) => r.ok), bad = res.filter((r) => !r.ok);
      closeModal();
      toast(`Removed ${ok.length} folder${ok.length === 1 ? "" : "s"}, ${fmt(ok.reduce((s, r) => s + r.freed, 0))}` + (bad.length ? `; ${bad.length} couldn't be removed (${bad[0].error})` : ""));
      loadDrives();
    } catch (e) { closeModal(); toast(String(e)); }
  };
}

// ---------- storage: path bar, history, live scan ----------
let hist = [], hidx = -1;
let scanId = 0;
let cur = { path: "", items: [], total: 0, loading: false, error: "" };
const scanCache = new Map();
let view = store.get("view", "list");
let renderTimer = null;

function normalizePath(p) {
  p = p.trim().replace(/\//g, "\\");
  if (/^[A-Za-z]:$/.test(p)) p += "\\";
  if (p.length > 3) p = p.replace(/\\+$/, "");
  return p;
}
function parentOf(p) {
  if (/^[A-Za-z]:\\?$/.test(p)) return p;
  const q = p.slice(0, p.lastIndexOf("\\"));
  return /^[A-Za-z]:$/.test(q) ? q + "\\" : q;
}
function go(path, push = true, force = false) {
  path = normalizePath(path);
  if (!path) return;
  if (push) { hist = hist.slice(0, hidx + 1); hist.push(path); hidx = hist.length - 1; }
  loadPath(path, force);
  updatePathBar();
}
function updatePathBar() {
  $("#st-path").value = cur.path || (hist[hidx] || "");
  $("#st-back").disabled = hidx <= 0;
  $("#st-fwd").disabled = hidx >= hist.length - 1;
  const p = hist[hidx] || "";
  $("#st-up").disabled = !p || parentOf(p) === p;
  const letter = p.slice(0, 1).toUpperCase();
  if (drives.find((d) => d.letter === letter)) $("#st-drive").value = letter;
}
async function loadPath(path, force) {
  const id = ++scanId;
  cur = { path, items: [], total: 0, loading: true, error: "" };
  if (!force && scanCache.has(path)) {
    cur.items = scanCache.get(path); cur.loading = false; cur.cached = true;
    renderStorage(); updatePathBar(); return;
  }
  renderStorage();
  try {
    const res = await invoke("scan_folder", { path, scanId: id });
    if (id !== scanId) return;
    cur.items = res; cur.loading = false;
    scanCache.set(path, res);
  } catch (e) {
    if (id !== scanId || String(e).includes("cancelled")) return;
    cur.loading = false; cur.error = String(e);
  }
  renderStorage(); updatePathBar();
}
listen("scan-item", (ev) => {
  const p = ev.payload;
  if (p.scan_id !== scanId || !cur.loading) return;
  cur.items.push(p.entry); cur.total = p.total;
  if (!renderTimer) renderTimer = setTimeout(() => { renderTimer = null; renderStorage(); }, 180);
});

function renderStorage() {
  const items = [...cur.items].sort((a, b) => b.size - a.size);
  const total = items.reduce((s, i) => s + i.size, 0);
  $("#st-status").innerHTML = cur.loading
    ? `<span class="spin"></span>Measured ${items.length} of ${cur.total || "…"} items · ${fmt(total)} so far`
    : cur.error ? "" : `${items.length} item${items.length === 1 ? "" : "s"} · ${fmt(total)} total` + (cur.cached ? " · cached, press Refresh to rescan" : "");
  const list = $("#st-list");
  if (cur.error) { list.innerHTML = `<div class="pad muted">${esc(cur.error)}</div>`; return; }
  if (!items.length) { list.innerHTML = `<div class="pad muted">${cur.loading ? "Starting scan…" : "Empty folder"}</div>`; return; }
  list._items = items;
  if (view === "tree") renderTree(items, list);
  else renderList(items, list);
}
function renderList(items, list) {
  const max = Math.max(1, ...items.map((i) => i.size));
  list.innerHTML = items.map((it, i) => `<div class="row ${it.is_dir ? "clickable" : ""}" ${it.is_dir ? `data-open="${i}"` : ""}>
    <div class="grow"><div class="title">${esc(it.name)}${it.is_dir ? "" : ' <span class="small muted">file</span>'}</div>
    <div class="barwrap"><div class="bar" style="width:${Math.max(1, (it.size / max) * 100)}%"></div></div></div>
    <div class="size">${fmt(it.size)}</div>
    <button class="btn small-btn" data-reveal="${i}" title="Show in File Explorer">Show</button></div>`).join("");
}

// squarified treemap
function squarify(nodes, x, y, w, h) {
  const total = nodes.reduce((s, n) => s + n.v, 0);
  const scale = (w * h) / total;
  const list = nodes.map((n) => ({ n, a: n.v * scale }));
  const out = [];
  let row = [];
  const worst = (r, side) => {
    const s = r.reduce((t, q) => t + q.a, 0);
    const mx = Math.max(...r.map((q) => q.a)), mn = Math.min(...r.map((q) => q.a));
    return Math.max((side * side * mx) / (s * s), (s * s) / (side * side * mn));
  };
  const layout = (r) => {
    const s = r.reduce((t, q) => t + q.a, 0);
    if (w >= h) {
      const rw = s / h; let cy = y;
      for (const q of r) { const rh = q.a / rw; out.push({ n: q.n, x, y: cy, w: rw, h: rh }); cy += rh; }
      x += rw; w -= rw;
    } else {
      const rh = s / w; let cx = x;
      for (const q of r) { const rw = q.a / rh; out.push({ n: q.n, x: cx, y, w: rw, h: rh }); cx += rw; }
      y += rh; h -= rh;
    }
  };
  for (const q of list) {
    const side = Math.min(w, h);
    if (!row.length || worst([...row, q], side) <= worst(row, side)) row.push(q);
    else { layout(row); row = [q]; }
  }
  if (row.length) layout(row);
  return out;
}
const PALETTE = ["#993C1D", "#185FA5", "#0F6E56", "#534AB7", "#854F0B", "#993556", "#3B6D11", "#5F5E5A"];
function renderTree(items, list) {
  const top = items.filter((i) => i.size > 0).slice(0, 40);
  if (!top.length) { list.innerHTML = '<div class="pad muted">Nothing measurable here</div>'; return; }
  const W = Math.max(300, (list.clientWidth || 700)), H = 420;
  const rects = squarify(top.map((it) => ({ it, v: it.size })), 0, 0, W, H);
  list.innerHTML = `<div class="tree">${rects.map((r, k) => {
    const it = r.n.it, idx = items.indexOf(it);
    const label = r.w > 70 && r.h > 38 ? `<b>${esc(it.name)}</b>${fmt(it.size)}` : "";
    return `<div class="tile ${it.is_dir ? "dir" : ""}" ${it.is_dir ? `data-open="${idx}"` : ""} title="${esc(it.name)} · ${fmt(it.size)}"
      style="left:${r.x}px;top:${r.y}px;width:${r.w}px;height:${r.h}px;background:${it.is_dir ? PALETTE[k % PALETTE.length] : "#6b6a65"}">${label}</div>`;
  }).join("")}</div><div class="small muted pad">Showing the ${top.length} largest items. Click a block to open that folder.</div>`;
}

$("#st-list").addEventListener("click", async (e) => {
  const items = $("#st-list")._items || [];
  const rv = e.target.closest("[data-reveal]");
  if (rv) {
    try { await invoke("reveal_path", { path: items[Number(rv.dataset.reveal)].path }); } catch (err) { toast(String(err)); }
    return;
  }
  const r = e.target.closest("[data-open]");
  if (r) go(items[Number(r.dataset.open)].path);
});
$("#st-back").onclick = () => { if (hidx > 0) { hidx--; go(hist[hidx], false); } };
$("#st-fwd").onclick = () => { if (hidx < hist.length - 1) { hidx++; go(hist[hidx], false); } };
$("#st-up").onclick = () => { const p = hist[hidx]; if (p && parentOf(p) !== p) go(parentOf(p)); };
$("#st-go").onclick = () => go($("#st-path").value);
$("#st-path").addEventListener("keydown", (e) => { if (e.key === "Enter") go($("#st-path").value); });
$("#st-drive").addEventListener("change", (e) => go(e.target.value + ":\\"));
$("#st-refresh").onclick = () => { const p = hist[hidx]; if (p) { scanCache.delete(p); go(p, false, true); } };
function setView(v) {
  view = v; store.set("view", v);
  $("#view-list").classList.toggle("on", v === "list");
  $("#view-tree").classList.toggle("on", v === "tree");
  if (cur.path) renderStorage();
}
$("#view-list").onclick = () => setView("list");
$("#view-tree").onclick = () => setView("tree");
window.addEventListener("resize", () => { if (view === "tree" && !$("#page-storage").classList.contains("hidden")) renderStorage(); });
setView(view);

// ---------- files: large files and duplicates ----------
let mode = "large";
let fileRes = { hits: [], groups: [] };
let scanning = false;

function setupFiles() {
  const opts = [`<option value="mine">My folders (Desktop, Documents, Downloads…)</option>`, `<option value="profile">Whole user profile</option>`];
  drives.forEach((d) => opts.push(`<option value="drive:${d.letter}">Whole drive ${d.letter}:</option>`));
  opts.push(`<option value="custom">Custom folder…</option>`);
  $("#fl-scope").innerHTML = opts.join("");
  fillMin();
}
function fillMin() {
  const sizes = mode === "large" ? [[50, "50 MB"], [100, "100 MB"], [500, "500 MB"], [1024, "1 GB"]] : [[1, "1 MB"], [10, "10 MB"], [50, "50 MB"], [100, "100 MB"]];
  const def = mode === "large" ? 100 : 10;
  $("#fl-min").innerHTML = sizes.map(([v, l]) => `<option value="${v}" ${v === def ? "selected" : ""}>Bigger than ${l}</option>`).join("");
}
$("#fl-scope").addEventListener("change", () => $("#fl-custom").classList.toggle("hidden", $("#fl-scope").value !== "custom"));
function setMode(m) {
  mode = m;
  $("#mode-large").classList.toggle("on", m === "large");
  $("#mode-dups").classList.toggle("on", m === "dups");
  fillMin();
  $("#fl-results").innerHTML = ""; $("#fl-status").textContent = ""; $("#fl-actions").classList.add("hidden");
  $("#fl-auto").classList.toggle("hidden", m !== "dups");
}
$("#mode-large").onclick = () => setMode("large");
$("#mode-dups").onclick = () => setMode("dups");
setMode("large");

function scopeRoots() {
  const s = $("#fl-scope").value;
  if (s === "mine") return known;
  if (s === "profile") return [userDir];
  if (s.startsWith("drive:")) return [s.slice(6) + ":\\"];
  const c = normalizePath($("#fl-custom").value);
  return c ? [c] : [];
}
listen("files-progress", (ev) => { if (scanning) $("#fl-status").innerHTML = `<span class="spin"></span>${esc(ev.payload)}`; });

$("#fl-scan").onclick = async () => {
  const roots = scopeRoots();
  if (!roots.length) { toast("Choose a folder to scan first"); return; }
  scanning = true;
  $("#fl-scan").classList.add("hidden"); $("#fl-cancel").classList.remove("hidden");
  $("#fl-results").innerHTML = ""; $("#fl-actions").classList.add("hidden");
  $("#fl-status").innerHTML = '<span class="spin"></span>Starting scan…';
  const min = Number($("#fl-min").value);
  try {
    if (mode === "large") { fileRes.hits = await invoke("find_large_files", { roots, minMb: min }); renderLarge(); }
    else { fileRes.groups = await invoke("find_duplicates", { roots, minMb: min }); renderDups(); }
  } catch (e) {
    $("#fl-status").textContent = String(e).includes("cancelled") ? "Scan cancelled" : String(e);
  }
  scanning = false;
  $("#fl-scan").classList.remove("hidden"); $("#fl-cancel").classList.add("hidden");
};
$("#fl-cancel").onclick = async () => { await invoke("cancel_files_scan"); };

function fileRow(path, size, checked) {
  const name = path.slice(path.lastIndexOf("\\") + 1);
  return `<label class="chk"><input type="checkbox" data-path="${esc(path)}" ${checked ? "checked" : ""} />
    <div class="grow"><div class="title">${esc(name)}</div><div class="small muted p">${esc(path)}</div></div>
    <div class="size">${fmt(size)}</div><button class="btn small-btn" data-show="${esc(path)}">Show</button></label>`;
}
function renderLarge() {
  const h = fileRes.hits;
  $("#fl-status").textContent = h.length ? `${h.length} large file${h.length === 1 ? "" : "s"} · ${fmt(h.reduce((s, x) => s + x.size, 0))} in total` : "No files that big were found";
  $("#fl-results").innerHTML = h.length ? `<div class="card list">${h.map((x) => fileRow(x.path, x.size, false)).join("")}</div>` : "";
  $("#fl-actions").classList.toggle("hidden", !h.length);
}
function renderDups() {
  const g = fileRes.groups;
  const wasted = g.reduce((s, x) => s + x.size * (x.files.length - 1), 0);
  $("#fl-status").textContent = g.length ? `${g.length} duplicate group${g.length === 1 ? "" : "s"} · about ${fmt(wasted)} could be freed` : "No duplicates found";
  $("#fl-results").innerHTML = g.map((x, gi) => `<div class="grp" data-g="${gi}"><div class="grp-h"><b>${x.files.length} copies</b> · ${fmt(x.size)} each · ${fmt(x.size * (x.files.length - 1))} wasted</div>${x.files.map((f) => fileRow(f, x.size, false)).join("")}</div>`).join("");
  $("#fl-actions").classList.toggle("hidden", !g.length);
}
$("#fl-results").addEventListener("click", async (e) => {
  const b = e.target.closest("[data-show]");
  if (b) { e.preventDefault(); try { await invoke("reveal_path", { path: b.dataset.show }); } catch (err) { toast(String(err)); } }
});
$("#fl-auto").onclick = () => {
  document.querySelectorAll("#fl-results .grp").forEach((grp) => {
    grp.querySelectorAll("input[type=checkbox]").forEach((c, i) => { c.checked = i > 0; });
  });
};
$("#fl-trash").onclick = () => {
  const boxes = [...document.querySelectorAll("#fl-results input:checked")];
  if (!boxes.length) { toast("Tick the files you want to remove first"); return; }
  if (mode === "dups") {
    for (const grp of document.querySelectorAll("#fl-results .grp")) {
      const all = grp.querySelectorAll("input[type=checkbox]");
      if (all.length && [...all].every((c) => c.checked)) { toast("Keep at least one copy in every group"); return; }
    }
  }
  const paths = boxes.map((c) => c.dataset.path);
  modal(`<h2>Move ${paths.length} file${paths.length > 1 ? "s" : ""} to the Recycle Bin?</h2>
    <p class="muted">You can restore them from the Recycle Bin until you empty it. Space comes back once it's emptied.</p>
    <div class="btns"><button class="btn" id="m-cancel">Cancel</button><button class="btn danger" id="m-go">Move to Recycle Bin</button></div>`);
  $("#m-cancel").onclick = closeModal;
  $("#m-go").onclick = async () => {
    modal(`<h2>Moving files…</h2><p class="muted"><span class="spin"></span>One moment.</p>`);
    try {
      const res = await invoke("trash_files", { paths });
      const ok = res.filter((r) => r.ok), bad = res.filter((r) => !r.ok);
      closeModal();
      const gone = new Set(ok.map((r) => r.path));
      fileRes.hits = fileRes.hits.filter((x) => !gone.has(x.path));
      fileRes.groups = fileRes.groups.map((g) => ({ ...g, files: g.files.filter((f) => !gone.has(f)) })).filter((g) => g.files.length > 1);
      mode === "large" ? renderLarge() : renderDups();
      toast(`Moved ${ok.length} file${ok.length === 1 ? "" : "s"} (${fmt(ok.reduce((s, r) => s + r.freed, 0))})` + (bad.length ? `; ${bad.length} couldn't be moved: ${bad[0].error}` : ""));
    } catch (err) { closeModal(); toast(String(err)); }
  };
};

// ---------- cleanup ----------
async function loadCaches() {
  loaded.cleanup = true;
  try {
    caches = await invoke("cache_targets");
    const total = caches.filter((c) => c.default_on).reduce((s, c) => s + c.size, 0);
    $("#reclaim").textContent = total ? "about " + fmt(total) : "Nothing to clean";
    $("#cache-list").className = "";
    $("#cache-list").innerHTML = caches.length ? caches.map((c) => `<label class="row"><input type="checkbox" ${c.default_on ? "checked" : ""} data-id="${esc(c.id)}" />
      <div class="grow"><div class="title">${esc(c.label)}</div><div class="small muted">${esc(c.note)}</div></div><div class="size">${fmt(c.size)}</div></label>`).join("") : '<div class="muted pad">Nothing to clean right now</div>';
  } catch (e) { loaded.cleanup = false; $("#cache-list").textContent = String(e); }
}
$("#refresh-cleanup").addEventListener("click", () => { $("#cache-list").className = "muted pad"; $("#cache-list").innerHTML = '<span class="spin"></span>Measuring…'; loadCaches(); });
$("#clean-btn").onclick = () => {
  const ids = [...document.querySelectorAll("#cache-list input:checked")].map((c) => c.dataset.id);
  if (!ids.length) { toast("Select something to clean first"); return; }
  const size = caches.filter((c) => ids.includes(c.id)).reduce((s, c) => s + c.size, 0);
  modal(`<h2>Clean ${fmt(size)} of cache?</h2><p class="muted">These files are permanently deleted. Apps rebuild them when needed, and files in use are skipped.</p>
    <div class="btns"><button class="btn" id="m-cancel">Cancel</button><button class="btn danger" id="m-go">Clean</button></div>`);
  $("#m-cancel").onclick = closeModal;
  $("#m-go").onclick = async () => {
    modal(`<h2>Cleaning…</h2><p class="muted"><span class="spin"></span>This can take a minute.</p>`);
    try {
      const r = await invoke("clean_cache", { ids });
      closeModal();
      toast(`Freed ${fmt(r.freed)}` + (r.skipped ? `; ${r.skipped} item${r.skipped > 1 ? "s" : ""} in use were skipped` : ""));
      loadCaches(); loadDrives();
    } catch (e) { closeModal(); toast(String(e)); }
  };
};

// ---------- boot ----------
(async function init() {
  try { userDir = await invoke("user_profile"); } catch (_) {}
  try { known = await invoke("known_folders"); } catch (_) {}
  await loadDrives();
  loadApps();
  loadCaches();
})();

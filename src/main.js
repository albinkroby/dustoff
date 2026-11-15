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
const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
const hue = (s) => [...s].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7);
const errText = (e) => String((e && e.message) || e).replace(/^Error:\s*/, "");
const store = {
  get(k, d) { try { return localStorage.getItem(k) ?? d; } catch (_) { return d; } },
  set(k, v) { try { localStorage.setItem(k, v); } catch (_) {} },
};

// ---------- icons ----------
const IC = {
  back: '<path d="M19 12H5"/><path d="m12 19-7-7 7-7"/>',
  forward: '<path d="M5 12h14"/><path d="m12 5 7 7-7 7"/>',
  up: '<path d="m18 15-6-6-6 6"/>',
  refresh: '<path d="M21 12a9 9 0 1 1-3-6.7"/><path d="M21 3v6h-6"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/>',
  list: '<path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"/>',
  treemap: '<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M12 3v18M12 12h9"/>',
  file: '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/>',
  copy: '<rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h10"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  trash: '<path d="M3 6h18"/><path d="M8 6V4h8v2"/><path d="m6 6 1 14h10l1-14"/>',
  external: '<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
  success: '<circle cx="12" cy="12" r="9"/><path d="m8 12 3 3 5-6"/>',
  error: '<circle cx="12" cy="12" r="9"/><path d="m9 9 6 6M15 9l-6 6"/>',
  warning: '<path d="M12 3 2 20h20z"/><path d="M12 10v4M12 17h.01"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/>',
  x: '<path d="M6 6l12 12M18 6 6 18"/>',
  chevron: '<path d="m6 9 6 6 6-6"/>',
  drive: '<rect x="3" y="14" width="18" height="6" rx="2"/><path d="M3 14l2.5-8h13L21 14"/><path d="M7 17h.01"/>',
  check: '<path d="m5 13 4 4 10-11"/>',
  shield: '<path d="M12 3 4 6v6c0 5 3.5 8 8 9 4.5-1 8-4 8-9V6z"/><path d="m9 12 2 2 4-4"/>',
  uninstall: '<path d="M4 7h16"/><path d="M9 7V4h6v3"/><path d="m6 7 1 13h10l1-13"/><path d="m10 11 4 4M14 11l-4 4"/>',
};
const svg = (n, s = 16) => `<svg class="ic" width="${s}" height="${s}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${IC[n]}</svg>`;
function decorateIcons(root = document) {
  root.querySelectorAll("[data-icon]").forEach((el) => {
    if (el.dataset.iconDone) return;
    el.dataset.iconDone = "1";
    el.insertAdjacentHTML("afterbegin", svg(el.dataset.icon, el.dataset.bare ? 18 : 15));
  });
}

// ---------- notifications ----------
const toastBox = $("#toasts");
function notify({ type = "info", title, text = "", timeout, action }) {
  const t = document.createElement("div");
  t.className = "toast " + type;
  t.setAttribute("role", type === "error" ? "alert" : "status");
  t.innerHTML = `<div class="t-ic">${svg(type, 18)}</div>
    <div class="t-body"><div class="t-title">${esc(title)}</div>${text ? `<div class="t-text">${esc(text)}</div>` : ""}${action ? `<button class="t-act">${esc(action.label)}</button>` : ""}</div>
    <button class="t-x" aria-label="Dismiss">${svg("x", 14)}</button>`;
  const ms = timeout ?? (type === "error" ? 10000 : type === "warning" ? 8000 : 4500);
  let timer;
  const close = () => { clearTimeout(timer); t.classList.add("out"); setTimeout(() => t.remove(), 200); };
  $(".t-x", t).onclick = close;
  if (action) $(".t-act", t).onclick = () => { action.onClick(); close(); };
  t.addEventListener("mouseenter", () => clearTimeout(timer));
  t.addEventListener("mouseleave", () => { timer = setTimeout(close, 2000); });
  timer = setTimeout(close, ms);
  toastBox.appendChild(t);
  while (toastBox.children.length > 4) toastBox.firstChild.remove();
}
const toast = {
  success: (title, text, o) => notify({ type: "success", title, text, ...o }),
  error: (title, text, o) => notify({ type: "error", title, text, ...o }),
  warning: (title, text, o) => notify({ type: "warning", title, text, ...o }),
  info: (title, text, o) => notify({ type: "info", title, text, ...o }),
};

function modal(html) { $("#modal-box").innerHTML = html; $("#modal").classList.remove("hidden"); }
function closeModal() { $("#modal").classList.add("hidden"); }

function showFailures(title, items, note = "These items weren't removed. They may be in use, protected, or need administrator rights.") {
  modal(`<h2>${esc(title)}</h2>
    <p class="muted">${esc(note)}</p>
    <div class="fail-list">${items.map((r) => `<div class="fail"><div class="p">${esc(r.path)}</div><div class="small muted">${esc(r.error || "Unknown error")}</div></div>`).join("")}</div>
    <div class="btns"><button class="btn primary" id="m-ok">Close</button></div>`);
  $("#m-ok").onclick = closeModal;
}

// report the outcome of a removal batch
function reportRemoval(noun, res, where) {
  const ok = res.filter((r) => r.ok), bad = res.filter((r) => !r.ok);
  const freed = ok.reduce((s, r) => s + r.freed, 0);
  const failAction = bad.length ? { label: "View details", onClick: () => showFailures(`Couldn't remove ${plural(bad.length, noun, noun + "s")}`, bad) } : null;
  if (ok.length && !bad.length) toast.success(`${plural(ok.length, noun, noun + "s")} removed`, `${fmt(freed)} ${where}.`);
  else if (ok.length) toast.warning(`${plural(ok.length, noun, noun + "s")} removed, ${bad.length} failed`, `${fmt(freed)} ${where}. Some items couldn't be removed.`, { action: failAction });
  else toast.error("Nothing was removed", `${plural(bad.length, noun, noun + "s")} couldn't be removed.`, { action: failAction });
  return { ok, bad, freed };
}

// ---------- clipboard and explorer ----------
async function copyText(text, label = "Path") {
  try { await navigator.clipboard.writeText(text); toast.success(`${label} copied`, text, { timeout: 2500 }); }
  catch (_) { toast.error("Couldn't copy", "Clipboard access was blocked."); }
}
async function reveal(path) {
  try { await invoke("reveal_path", { path }); }
  catch (e) { toast.error("Couldn't open File Explorer", errText(e)); }
}

let apps = [];
let sortBy = "size";
let loaded = { apps: false, storage: false, cleanup: false, files: false };
let caches = [];
let drives = [];
let curDrive = "C";
let userDir = "C:\\";
let known = [];

// ---------- custom dropdowns ----------
const selDesc = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value");
function enhanceSelect(sel) {
  if (sel._cs) return;
  const wrap = document.createElement("div");
  wrap.className = "cs";
  sel.parentNode.insertBefore(wrap, sel);
  wrap.appendChild(sel);
  sel.classList.add("cs-native");
  sel.tabIndex = -1;
  const btn = document.createElement("button");
  btn.type = "button"; btn.className = "cs-btn";
  btn.setAttribute("aria-haspopup", "listbox");
  if (sel.getAttribute("aria-label")) btn.setAttribute("aria-label", sel.getAttribute("aria-label"));
  btn.innerHTML = `<span class="cs-label"></span>${svg("chevron", 14)}`;
  const menu = document.createElement("div");
  menu.className = "cs-menu hidden"; menu.setAttribute("role", "listbox");
  wrap.append(btn, menu);
  let hl = -1;

  const sync = () => {
    const o = sel.options[sel.selectedIndex];
    $(".cs-label", btn).textContent = o ? o.textContent : "";
    menu.innerHTML = [...sel.options].map((op, i) => `<button type="button" class="cs-opt ${i === sel.selectedIndex ? "on" : ""}" role="option" data-i="${i}">${esc(op.textContent)}${i === sel.selectedIndex ? svg("check", 14) : ""}</button>`).join("");
  };
  const open = () => {
    document.querySelectorAll(".cs.open").forEach((c) => c !== wrap && c._close && c._close());
    menu.classList.remove("hidden"); wrap.classList.add("open"); btn.setAttribute("aria-expanded", "true");
    hl = sel.selectedIndex; mark();
  };
  const close = () => { menu.classList.add("hidden"); wrap.classList.remove("open"); btn.setAttribute("aria-expanded", "false"); };
  const mark = () => menu.querySelectorAll(".cs-opt").forEach((o, i) => o.classList.toggle("hl", i === hl));
  const choose = (i) => {
    sel.selectedIndex = i; sync(); close(); btn.focus();
    sel.dispatchEvent(new Event("change", { bubbles: true }));
  };
  wrap._close = close;
  btn.addEventListener("click", () => (menu.classList.contains("hidden") ? open() : close()));
  menu.addEventListener("click", (e) => { const o = e.target.closest(".cs-opt"); if (o) choose(Number(o.dataset.i)); });
  btn.addEventListener("keydown", (e) => {
    const n = sel.options.length;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (menu.classList.contains("hidden")) { open(); return; }
      hl = (hl + (e.key === "ArrowDown" ? 1 : -1) + n) % n; mark();
      menu.querySelectorAll(".cs-opt")[hl]?.scrollIntoView({ block: "nearest" });
    } else if ((e.key === "Enter" || e.key === " ") && !menu.classList.contains("hidden")) {
      e.preventDefault(); if (hl >= 0) choose(hl);
    } else if (e.key === "Escape" && !menu.classList.contains("hidden")) {
      e.preventDefault(); e.stopPropagation(); close();
    }
  });
  new MutationObserver(sync).observe(sel, { childList: true, subtree: true, attributes: true });
  Object.defineProperty(sel, "value", {
    get() { return selDesc.get.call(this); },
    set(v) { selDesc.set.call(this, v); sync(); },
  });
  sel._cs = true;
  sync();
}
document.addEventListener("click", (e) => {
  document.querySelectorAll(".cs.open").forEach((c) => { if (!c.contains(e.target)) c._close(); });
});

// ---------- context menu ----------
const ctx = $("#ctx");
function hideCtx() { ctx.classList.add("hidden"); ctx.innerHTML = ""; ctx._items = null; }
function showCtx(x, y, items) {
  ctx._items = items;
  ctx.innerHTML = items.map((it, i) => it === "-" ? '<div class="ctx-sep"></div>' :
    `<button class="ctx-item ${it.danger ? "danger" : ""}" role="menuitem" data-i="${i}" ${it.disabled ? "disabled" : ""}>${it.icon ? svg(it.icon, 16) : ""}<span>${esc(it.label)}</span></button>`).join("");
  ctx.classList.remove("hidden");
  const w = ctx.offsetWidth, h = ctx.offsetHeight;
  ctx.style.left = Math.max(6, Math.min(x, window.innerWidth - w - 6)) + "px";
  ctx.style.top = Math.max(6, Math.min(y, window.innerHeight - h - 6)) + "px";
  const first = ctx.querySelector(".ctx-item:not(:disabled)");
  if (first) first.focus();
}
ctx.addEventListener("click", (e) => {
  const b = e.target.closest(".ctx-item");
  if (!b || !ctx._items) return;
  const it = ctx._items[Number(b.dataset.i)];
  hideCtx();
  if (it && it.run) it.run();
});
ctx.addEventListener("keydown", (e) => {
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    const btns = [...ctx.querySelectorAll(".ctx-item:not(:disabled)")];
    const i = btns.indexOf(document.activeElement);
    btns[(i + (e.key === "ArrowDown" ? 1 : -1) + btns.length) % btns.length]?.focus();
  }
});
document.addEventListener("mousedown", (e) => { if (!ctx.contains(e.target)) hideCtx(); });
window.addEventListener("blur", hideCtx);
window.addEventListener("resize", hideCtx);
document.addEventListener("scroll", hideCtx, true);

function menuFor(target) {
  const st = target.closest("#st-list [data-i]");
  if (st) {
    const it = ($("#st-list")._items || [])[Number(st.dataset.i)];
    if (!it) return null;
    return [
      it.is_dir && { label: "Open", icon: "folder", run: () => go(it.path) },
      { label: "Show in File Explorer", icon: "external", run: () => reveal(it.path) },
      { label: "Copy path", icon: "copy", run: () => copyText(it.path) },
      it.is_dir && "-",
      it.is_dir && { label: "Never touch this folder", icon: "shield", run: () => protect(it.path) },
    ].filter(Boolean);
  }
  const ap = target.closest("#apps-list [data-ai]");
  if (ap) {
    const a = apps[Number(ap.dataset.ai)];
    if (!a) return null;
    return [
      { label: "Uninstall + clean…", icon: "uninstall", danger: true, run: () => confirmUninstall(a) },
      "-",
      a.install_location && { label: "Show install folder", icon: "external", run: () => reveal(a.install_location) },
      { label: "Copy app name", icon: "copy", run: () => copyText(a.name, "App name") },
    ].filter(Boolean);
  }
  const fr = target.closest("#fl-results [data-fpath]");
  if (fr) {
    const p = fr.dataset.fpath, box = $("input[type=checkbox]", fr);
    return [
      { label: box && box.checked ? "Untick" : "Tick for removal", icon: "check", run: () => { if (box) box.checked = !box.checked; } },
      "-",
      { label: "Show in File Explorer", icon: "external", run: () => reveal(p) },
      { label: "Copy path", icon: "copy", run: () => copyText(p) },
    ];
  }
  return null;
}
// Replace the browser's default right-click menu (Reload, Save as, Print, Inspect) with our own.
document.addEventListener("contextmenu", (e) => {
  if (e.target.closest("input[type=text], input[type=search], textarea")) return; // keep cut/copy/paste in text fields
  e.preventDefault();
  if (!$("#modal").classList.contains("hidden")) { hideCtx(); return; }
  const items = menuFor(e.target);
  if (items && items.length) showCtx(e.clientX, e.clientY, items); else hideCtx();
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    if (!ctx.classList.contains("hidden")) { hideCtx(); return; }
    if (!$("#modal").classList.contains("hidden") && ($("#m-cancel") || $("#m-ok"))) closeModal();
  }
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
  hideCtx();
  document.querySelectorAll(".page").forEach((p) => p.classList.add("hidden"));
  $("#page-" + page).classList.remove("hidden");
  document.querySelectorAll(".nav").forEach((n) => n.classList.toggle("on", n.dataset.page === page));
  if (page === "apps" && !loaded.apps) loadApps();
  if (page === "storage" && !loaded.storage) { loaded.storage = true; go(userDir); }
  if (page === "cleanup" && !loaded.cleanup) loadCaches();
  if (page === "files" && !loaded.files) { loaded.files = true; setupFiles(); }
  if (page === "safety") loadSafety();
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
  } catch (e) { toast.error("Couldn't read your drives", errText(e)); }
}
function renderDrives() {
  $("#drive-chips").innerHTML = drives.length > 1
    ? drives.map((d) => {
        const pct = Math.round(((d.total - d.free) / d.total) * 100);
        return `<button class="chip ${d.letter === curDrive ? "on" : ""}" data-drive="${d.letter}">${svg("drive", 15)}<span>${d.letter}:${d.removable ? " (removable)" : ""}</span><span class="chip-pct">${pct}%</span></button>`;
      }).join("")
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

// ---------- app icons ----------
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

// ---------- apps ----------
function dateLabel(iso) {
  const d = new Date(iso + "T00:00:00");
  return isNaN(d) ? iso : d.toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}
function agoLabel(iso) {
  const d = new Date(iso + "T00:00:00");
  if (isNaN(d)) return iso;
  const days = Math.floor((Date.now() - d.getTime()) / 864e5);
  if (days <= 0) return "today";
  if (days === 1) return "yesterday";
  if (days < 31) return `${days} days ago`;
  if (days < 365) { const m = Math.floor(days / 30); return `${m} month${m > 1 ? "s" : ""} ago`; }
  const y = Math.floor(days / 365);
  return `${y} year${y > 1 ? "s" : ""} ago`;
}
function appRow(a, i, withButton) {
  const meta = [a.publisher, a.version].filter(Boolean).map(esc).join(" · ");
  return `<div class="row" ${withButton ? `data-ai="${i}"` : ""}><div class="avatar" data-ik="${esc(ikey(a))}" style="--h:${hue(a.name)}">${esc((a.name[0] || "?").toUpperCase())}</div>
    <div class="grow"><div class="title">${esc(a.name)}</div><div class="small muted title">${meta}</div></div>
    <div class="dates small muted">
      <div title="${a.installed_approx ? "Approximate: the date Windows last changed this app's entry" : "Install date reported by the app"}">${a.installed ? `${a.installed_approx ? "Updated" : "Installed"} ${esc(dateLabel(a.installed))}` : "Install date unknown"}</div>
      <div title="From Windows launch history. Apps never opened from Start or Explorer show as unknown.">${a.last_used ? `Last used ${esc(agoLabel(a.last_used))}` : "Last used unknown"}</div>
    </div>
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
  } catch (e) {
    loaded.apps = false;
    $("#apps-list").className = "muted pad";
    $("#apps-list").textContent = "Couldn't load your apps.";
    toast.error("Couldn't load your apps", errText(e));
  }
}
function renderApps() {
  const q = $("#app-search").value.trim().toLowerCase();
  const rows = apps.map((a, i) => ({ a, i })).filter(({ a }) => !q || a.name.toLowerCase().includes(q) || a.publisher.toLowerCase().includes(q));
  if (sortBy === "name") rows.sort((x, y) => x.a.name.localeCompare(y.a.name));
  else if (sortBy === "installed") rows.sort((x, y) => (x.a.installed || "9999").localeCompare(y.a.installed || "9999"));
  else if (sortBy === "used") rows.sort((x, y) => (x.a.last_used || "9999").localeCompare(y.a.last_used || "9999"));
  $("#app-count").textContent = plural(rows.length, "app", "apps") + (q ? " match" : " installed");
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
    catch (e) { closeModal(); toast.error(`Couldn't uninstall ${app.name}`, errText(e)); return; }
    modal(`<h2>Looking for leftovers…</h2><p class="muted"><span class="spin"></span>Scanning app data folders.</p>`);
    try {
      const left = await invoke("find_leftovers", { name: app.name, publisher: app.publisher, installLocation: app.install_location });
      loaded.apps = false; loadApps(); loadDrives();
      reviewLeftovers(app, left);
    } catch (e) {
      closeModal();
      loaded.apps = false; loadApps();
      toast.warning(`${app.name} wasn't removed`, errText(e));
    }
  };
}
function reviewLeftovers(app, left) {
  if (!left.length) {
    closeModal();
    toast.success(`${app.name} uninstalled`, "No leftover folders were found.");
    return;
  }
  const total = left.reduce((s, l) => s + l.size, 0);
  modal(`<h2>Leftover data for ${esc(app.name)}</h2>
    <p class="muted">Found ${fmt(total)} in ${plural(left.length, "folder", "folders")}. Untick anything you want to keep.</p>
    <div id="lo-list">${left.map((l, i) => `<label class="chk"><input type="checkbox" checked data-i="${i}" /><div class="grow"><div class="p">${esc(l.path)}</div><div class="small muted">${fmt(l.size)}</div></div></label>`).join("")}</div>
    <label class="opt"><input type="checkbox" id="lo-perm" /> Delete permanently (otherwise moved to the Recycle Bin)</label>
    <div class="btns"><button class="btn" id="m-cancel">Keep all</button><button class="btn danger" id="m-del">Remove selected</button></div>`);
  $("#m-cancel").onclick = () => { closeModal(); toast.info(`${app.name} uninstalled`, "Leftover folders were kept."); };
  $("#m-del").onclick = async () => {
    const paths = [...document.querySelectorAll("#lo-list input:checked")].map((c) => left[Number(c.dataset.i)].path);
    if (!paths.length) { closeModal(); return; }
    const perm = $("#lo-perm").checked;
    openProgress(`Removing leftovers of ${app.name}`, "Starting…");
    try {
      const res = await invoke("delete_paths", { paths, permanent: perm, title: `Leftovers of ${app.name}` });
      closeModal();
      reportRemoval("folder", res, perm ? "freed" : "moved to the Recycle Bin");
      loadDrives(); refreshSafetyIfOpen();
    } catch (e) { closeModal(); toast.error("Couldn't remove leftovers", errText(e)); }
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
    cur.loading = false; cur.error = errText(e);
    toast.error("Couldn't open that folder", cur.error);
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
    : cur.error ? "" : `${plural(items.length, "item", "items")} · ${fmt(total)} total` + (cur.cached ? " · cached, press Refresh to rescan" : "");
  const list = $("#st-list");
  if (cur.error) { list.innerHTML = `<div class="pad muted">${esc(cur.error)}</div>`; return; }
  if (!items.length) { list.innerHTML = `<div class="pad muted">${cur.loading ? "Starting scan…" : "Empty folder"}</div>`; return; }
  list._items = items;
  if (view === "tree") renderTree(items, list);
  else renderList(items, list);
}
function renderList(items, list) {
  const max = Math.max(1, ...items.map((i) => i.size));
  list.innerHTML = items.map((it, i) => `<div class="row ${it.is_dir ? "clickable" : ""}" data-i="${i}" ${it.is_dir ? `data-open="${i}"` : ""}>
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
    return `<div class="tile ${it.is_dir ? "dir" : ""}" data-i="${idx}" ${it.is_dir ? `data-open="${idx}"` : ""} title="${esc(it.name)} · ${fmt(it.size)}"
      style="left:${r.x}px;top:${r.y}px;width:${r.w}px;height:${r.h}px;background:${it.is_dir ? PALETTE[k % PALETTE.length] : "#6b6a65"}">${label}</div>`;
  }).join("")}</div><div class="small muted pad">Showing the ${top.length} largest items. Click a block to open that folder, or right-click for more options.</div>`;
}

$("#st-list").addEventListener("click", async (e) => {
  const items = $("#st-list")._items || [];
  const rv = e.target.closest("[data-reveal]");
  if (rv) { reveal(items[Number(rv.dataset.reveal)].path); return; }
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
  if (!roots.length) { toast.warning("Choose a folder first", "Pick where to look, or type a custom folder path."); return; }
  scanning = true;
  $("#fl-scan").classList.add("hidden"); $("#fl-cancel").classList.remove("hidden");
  $("#fl-results").innerHTML = ""; $("#fl-actions").classList.add("hidden");
  $("#fl-status").innerHTML = '<span class="spin"></span>Starting scan…';
  const min = Number($("#fl-min").value);
  try {
    if (mode === "large") { fileRes.hits = await invoke("find_large_files", { roots, minMb: min }); renderLarge(); }
    else { fileRes.groups = await invoke("find_duplicates", { roots, minMb: min }); renderDups(); }
  } catch (e) {
    if (String(e).includes("cancelled")) { $("#fl-status").textContent = "Scan cancelled"; toast.info("Scan cancelled"); }
    else { $("#fl-status").textContent = ""; toast.error("The scan didn't finish", errText(e)); }
  }
  scanning = false;
  $("#fl-scan").classList.remove("hidden"); $("#fl-cancel").classList.add("hidden");
};
$("#fl-cancel").onclick = async () => { await invoke("cancel_files_scan"); };

function fileRow(path, size, checked) {
  const name = path.slice(path.lastIndexOf("\\") + 1);
  return `<label class="chk" data-fpath="${esc(path)}"><input type="checkbox" data-path="${esc(path)}" ${checked ? "checked" : ""} />
    <div class="grow"><div class="title">${esc(name)}</div><div class="small muted p">${esc(path)}</div></div>
    <div class="size">${fmt(size)}</div><button class="btn small-btn" data-show="${esc(path)}">Show</button></label>`;
}
function renderLarge() {
  const h = fileRes.hits;
  $("#fl-status").textContent = h.length ? `${plural(h.length, "large file", "large files")} · ${fmt(h.reduce((s, x) => s + x.size, 0))} in total` : "No files that big were found";
  $("#fl-results").innerHTML = h.length ? `<div class="card list">${h.map((x) => fileRow(x.path, x.size, false)).join("")}</div>` : "";
  $("#fl-actions").classList.toggle("hidden", !h.length);
}
function renderDups() {
  const g = fileRes.groups;
  const wasted = g.reduce((s, x) => s + x.size * (x.files.length - 1), 0);
  $("#fl-status").textContent = g.length ? `${plural(g.length, "duplicate group", "duplicate groups")} · about ${fmt(wasted)} could be freed` : "No duplicates found";
  $("#fl-results").innerHTML = g.map((x, gi) => `<div class="grp" data-g="${gi}"><div class="grp-h"><b>${x.files.length} copies</b> · ${fmt(x.size)} each · ${fmt(x.size * (x.files.length - 1))} wasted</div>${x.files.map((f) => fileRow(f, x.size, false)).join("")}</div>`).join("");
  $("#fl-actions").classList.toggle("hidden", !g.length);
}
$("#fl-results").addEventListener("click", (e) => {
  const b = e.target.closest("[data-show]");
  if (b) { e.preventDefault(); reveal(b.dataset.show); }
});
$("#fl-auto").onclick = () => {
  document.querySelectorAll("#fl-results .grp").forEach((grp) => {
    grp.querySelectorAll("input[type=checkbox]").forEach((c, i) => { c.checked = i > 0; });
  });
  toast.info("Extra copies selected", "The first copy in each group is kept.", { timeout: 3000 });
};
$("#fl-trash").onclick = () => {
  const boxes = [...document.querySelectorAll("#fl-results input:checked")];
  if (!boxes.length) { toast.warning("Nothing selected", "Tick the files you want to remove first."); return; }
  if (mode === "dups") {
    for (const grp of document.querySelectorAll("#fl-results .grp")) {
      const all = grp.querySelectorAll("input[type=checkbox]");
      if (all.length && [...all].every((c) => c.checked)) { toast.warning("Keep at least one copy", "Every group needs one copy left. Untick one file in each group."); return; }
    }
  }
  const paths = boxes.map((c) => c.dataset.path);
  modal(`<h2>Move ${plural(paths.length, "file", "files")} to the Recycle Bin?</h2>
    <p class="muted">You can restore them from the Recycle Bin until you empty it. Space comes back once it's emptied.</p>
    <div class="btns"><button class="btn" id="m-cancel">Cancel</button><button class="btn danger" id="m-go">Move to Recycle Bin</button></div>`);
  $("#m-cancel").onclick = closeModal;
  $("#m-go").onclick = async () => {
    openProgress("Moving files to the Recycle Bin", "Starting…");
    try {
      const res = await invoke("trash_files", { paths, title: mode === "dups" ? "Duplicate files" : "Large files" });
      closeModal();
      refreshSafetyIfOpen();
      const { ok } = reportRemoval("file", res, "moved to the Recycle Bin");
      const gone = new Set(ok.map((r) => r.path));
      fileRes.hits = fileRes.hits.filter((x) => !gone.has(x.path));
      fileRes.groups = fileRes.groups.map((g) => ({ ...g, files: g.files.filter((f) => !gone.has(f)) })).filter((g) => g.files.length > 1);
      mode === "large" ? renderLarge() : renderDups();
    } catch (err) { closeModal(); toast.error("Couldn't move the files", errText(err)); }
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
  } catch (e) {
    loaded.cleanup = false;
    $("#cache-list").className = "muted pad";
    $("#cache-list").textContent = "Couldn't measure your caches.";
    toast.error("Couldn't measure your caches", errText(e));
  }
}
$("#refresh-cleanup").addEventListener("click", () => { $("#cache-list").className = "muted pad"; $("#cache-list").innerHTML = '<span class="spin"></span>Measuring…'; loadCaches(); });
// ---------- progress, smart clean, cleanup actions ----------
function openProgress(title, sub = "Working…") {
  modal(`<h2>${esc(title)}</h2><p class="muted pg-label" id="pg-label">${esc(sub)}</p>
    <div class="pbar"><div class="pfill" id="pg-fill" style="width:3%"></div></div>
    <div class="pmeta small muted"><span id="pg-count"></span><span id="pg-freed"></span></div>`);
}
listen("task-progress", (ev) => {
  const p = ev.payload, fill = $("#pg-fill");
  if (!fill) return;
  const pct = p.total ? Math.round((p.done / p.total) * 100) : 0;
  fill.style.width = Math.max(3, pct) + "%";
  $("#pg-label").textContent = p.label;
  $("#pg-count").textContent = p.total ? `${p.done} of ${p.total}` : "";
  $("#pg-freed").textContent = p.freed ? `${fmt(p.freed)} freed so far` : "";
});

function showCleanResult(r) {
  const skips = r.skipped > 0;
  let kind, head, sub;
  if (r.freed > 0 && !skips) { kind = "success"; head = `${fmt(r.freed)} freed`; sub = "Cleanup finished."; }
  else if (r.freed > 0) { kind = "warning"; head = `${fmt(r.freed)} freed`; sub = `${plural(r.skipped, "item was", "items were")} in use or protected and skipped. Close the related apps and try again for the rest.`; }
  else if (skips) { kind = "warning"; head = "Nothing could be cleaned"; sub = `${plural(r.skipped, "item was", "items were")} in use or protected. Close the related apps, or run Dustoff as administrator.`; }
  else { kind = "info"; head = "Nothing to clean"; sub = "Those locations were already empty."; }
  const rows = r.details.map((d) => `<div class="rrow"><span>${esc(d.label)}</span><span class="${d.freed ? "" : "muted"}">${d.freed ? fmt(d.freed) : "Nothing freed"}${d.skipped ? ` · ${d.skipped} skipped` : ""}</span></div>`).join("");
  modal(`<div class="result ${kind}"><div class="r-ic">${svg(kind, 28)}</div><div><h2>${esc(head)}</h2><p class="muted">${esc(sub)}</p></div></div>
    ${rows ? `<div class="rlist">${rows}</div>` : ""}
    <div class="btns"><button class="btn" id="m-act">View activity</button><button class="btn primary" id="m-ok">Done</button></div>`);
  $("#m-ok").onclick = closeModal;
  $("#m-act").onclick = () => { closeModal(); show("safety"); };
}

async function runClean(ids, title, smart = false) {
  openProgress(smart ? "Smart clean in progress" : "Cleaning…", "Starting…");
  try {
    const r = await invoke("clean_cache", { ids, title });
    showCleanResult(r);
    loadCaches(); loadDrives(); refreshSafetyIfOpen();
  } catch (e) { closeModal(); toast.error("Cleanup didn't finish", errText(e)); }
}

async function smartClean() {
  if (!caches.length) await loadCaches();
  const items = caches.filter((c) => c.default_on);
  if (!items.length) { toast.info("Nothing to clean", "No safe cache items were found right now."); return; }
  const total = items.reduce((s, c) => s + c.size, 0);
  modal(`<h2>Smart clean</h2>
    <p class="muted">Dustoff will clean only these safe items. Browser caches, app caches and the Recycle Bin are not touched, and folders you protected are skipped.</p>
    <div class="rlist">${items.map((c) => `<div class="rrow"><span>${esc(c.label)}</span><span>${fmt(c.size)}</span></div>`).join("")}</div>
    <div class="btns"><button class="btn" id="m-cancel">Cancel</button><button class="btn primary" id="m-go">Clean ${fmt(total)}</button></div>`);
  $("#m-cancel").onclick = closeModal;
  $("#m-go").onclick = () => runClean(items.map((c) => c.id), "Smart clean", true);
}
$("#smart-clean").addEventListener("click", smartClean);

$("#clean-btn").onclick = () => {
  const ids = [...document.querySelectorAll("#cache-list input:checked")].map((c) => c.dataset.id);
  if (!ids.length) { toast.warning("Nothing selected", "Select something to clean first."); return; }
  const size = caches.filter((c) => ids.includes(c.id)).reduce((s, c) => s + c.size, 0);
  modal(`<h2>Clean ${fmt(size)} of cache?</h2><p class="muted">These files are permanently deleted. Apps rebuild them when needed, and files in use are skipped.</p>
    <div class="btns"><button class="btn" id="m-cancel">Cancel</button><button class="btn danger" id="m-go">Clean</button></div>`);
  $("#m-cancel").onclick = closeModal;
  $("#m-go").onclick = () => runClean(ids, "Cache cleanup");
};

// ---------- safety: protected folders and activity log ----------
let safetyLoaded = false;
async function loadSafety() {
  safetyLoaded = true;
  await Promise.all([renderProtected(), renderHistory()]);
}
function refreshSafetyIfOpen() { if (safetyLoaded) loadSafety(); }

async function renderProtected() {
  try {
    const list = await invoke("protected_list");
    $("#prot-list")._items = list;
    $("#prot-list").innerHTML = list.length
      ? list.map((p, i) => `<div class="row"><div class="avatar sh">${svg("shield", 18)}</div><div class="grow"><div class="p">${esc(p)}</div></div><button class="btn small-btn" data-unprot="${i}">Remove</button></div>`).join("")
      : '<div class="muted pad small">No protected folders yet. Add one above, or right-click a folder in Storage.</div>';
  } catch (e) { toast.error("Couldn't load protected folders", errText(e)); }
}
async function protect(path) {
  try {
    await invoke("protect_add", { path });
    toast.success("Folder protected", path);
    refreshSafetyIfOpen();
    return true;
  } catch (e) { toast.error("Couldn't protect that folder", errText(e)); return false; }
}
$("#prot-add").onclick = async () => {
  const v = normalizePath($("#prot-path").value);
  if (!v) { toast.warning("Enter a folder path first"); return; }
  if (await protect(v)) $("#prot-path").value = "";
};
$("#prot-path").addEventListener("keydown", (e) => { if (e.key === "Enter") $("#prot-add").click(); });
$("#prot-list").addEventListener("click", async (e) => {
  const b = e.target.closest("[data-unprot]");
  if (!b) return;
  const p = ($("#prot-list")._items || [])[Number(b.dataset.unprot)];
  try { await invoke("protect_remove", { path: p }); toast.info("Protection removed", p); renderProtected(); }
  catch (err) { toast.error("Couldn't update the list", errText(err)); }
});

const HIST_ICON = { remove: "trash", files: "file", cache: "refresh", uninstall: "uninstall" };
async function renderHistory() {
  try {
    const h = await invoke("list_history");
    $("#hist-list")._items = h;
    $("#hist-list").innerHTML = h.length ? h.map((e, i) => {
      const when = new Date(e.ts * 1000).toLocaleString(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
      const recycled = e.items.filter((x) => x.recycled).length;
      const detail = e.kind === "uninstall" ? "Uninstalled"
        : `${fmt(e.freed)}${recycled ? ` · ${plural(recycled, "item", "items")} in the Recycle Bin` : " · permanently removed"}`;
      const badge = e.undone ? '<span class="badge">Restored</span>' : "";
      const btn = recycled && !e.undone ? `<button class="btn small-btn" data-undo="${i}">Restore</button>` : "";
      return `<div class="row"><div class="avatar sh">${svg(HIST_ICON[e.kind] || "info", 18)}</div><div class="grow"><div class="title">${esc(e.title)}</div><div class="small muted">${esc(when)} · ${esc(detail)}</div></div>${badge}${btn}</div>`;
    }).join("") : '<div class="muted pad small">Nothing has been removed yet.</div>';
  } catch (e) { toast.error("Couldn't load the activity log", errText(e)); }
}
$("#hist-list").addEventListener("click", async (e) => {
  const b = e.target.closest("[data-undo]");
  if (!b) return;
  const entry = ($("#hist-list")._items || [])[Number(b.dataset.undo)];
  b.disabled = true;
  try {
    const r = await invoke("undo_entry", { id: entry.id });
    const fails = r.failed.map((f) => ({ path: f.path, error: f.error }));
    const action = fails.length ? { label: "View details", onClick: () => showFailures("Couldn't restore some items", fails, "These items are no longer in the Recycle Bin, or something is in their way.") } : null;
    if (fails.length && r.restored) toast.warning(`${plural(r.restored, "item", "items")} restored, ${fails.length} couldn't be`, "Some items are no longer in the Recycle Bin.", { action });
    else if (fails.length) toast.error("Nothing was restored", fails[0].error, { action });
    else toast.success(`${plural(r.restored, "item", "items")} restored`, "They're back in their original folders.");
    loadDrives(); renderHistory();
  } catch (err) { toast.error("Couldn't restore", errText(err)); b.disabled = false; }
});
$("#hist-clear").onclick = () => {
  modal(`<h2>Clear the activity log?</h2><p class="muted">The list is erased. Files already in the Recycle Bin stay there, but you won't be able to restore them from Dustoff.</p>
    <div class="btns"><button class="btn" id="m-cancel">Cancel</button><button class="btn danger" id="m-go">Clear log</button></div>`);
  $("#m-cancel").onclick = closeModal;
  $("#m-go").onclick = async () => {
    try { await invoke("clear_history"); closeModal(); toast.info("Activity log cleared"); renderHistory(); }
    catch (e) { closeModal(); toast.error("Couldn't clear the log", errText(e)); }
  };
};
$("#refresh-safety").onclick = loadSafety;

// ---------- boot ----------
decorateIcons();
["#app-sort", "#st-drive", "#fl-scope", "#fl-min"].forEach((s) => enhanceSelect($(s)));
(async function init() {
  try { userDir = await invoke("user_profile"); } catch (_) {}
  try { known = await invoke("known_folders"); } catch (_) {}
  await loadDrives();
  loadApps();
  loadCaches();
})();

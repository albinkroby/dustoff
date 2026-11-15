use rayon::prelude::*;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::io::Read;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::Emitter;
use winreg::enums::*;
use winreg::RegKey;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ---------- cancellation ----------

static STORAGE_GEN: AtomicU64 = AtomicU64::new(0);
static FILES_GEN: AtomicU64 = AtomicU64::new(0);
static NO_CANCEL: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
struct Cancel {
    ctr: &'static AtomicU64,
    id: u64,
}

impl Cancel {
    fn none() -> Self {
        Cancel { ctr: &NO_CANCEL, id: 0 }
    }
    fn start(ctr: &'static AtomicU64) -> Self {
        let id = ctr.fetch_add(1, Ordering::SeqCst) + 1;
        Cancel { ctr, id }
    }
    fn cancelled(&self) -> bool {
        self.ctr.load(Ordering::Relaxed) != self.id
    }
}

// ---------- helpers ----------

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).map(PathBuf::from)
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// Parallel recursive size of a directory. Uses directory-entry metadata, so no extra syscall per file.
fn dir_size_par(p: &Path, c: Cancel) -> u64 {
    if c.cancelled() {
        return 0;
    }
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    let entries: Vec<std::fs::DirEntry> = rd.filter_map(|e| e.ok()).collect();
    entries
        .par_iter()
        .map(|e| {
            let Ok(ft) = e.file_type() else { return 0 };
            if ft.is_symlink() {
                return 0;
            }
            if ft.is_dir() {
                dir_size_par(&e.path(), c)
            } else {
                e.metadata().map(|m| m.len()).unwrap_or(0)
            }
        })
        .sum()
}

fn entry_size(p: &Path, c: Cancel) -> u64 {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_file() => m.len(),
        Ok(m) if m.is_dir() => dir_size_par(p, c),
        _ => 0,
    }
}

fn dir_size(p: &Path) -> u64 {
    entry_size(p, Cancel::none())
}

fn skip_dir(p: &Path) -> bool {
    let n = name_of(p).to_lowercase();
    if matches!(
        n.as_str(),
        "$recycle.bin" | "system volume information" | "windowsapps" | "node_modules" | ".git"
    ) {
        return true;
    }
    if n == "windows" {
        return p.parent().map(|x| x.parent().is_none()).unwrap_or(false);
    }
    false
}

/// Collect files of at least `min` bytes under `p`, walking in parallel.
fn collect_files(p: &Path, c: Cancel, min: u64) -> Vec<(PathBuf, u64)> {
    if c.cancelled() || skip_dir(p) {
        return vec![];
    }
    let Ok(rd) = std::fs::read_dir(p) else { return vec![] };
    let entries: Vec<std::fs::DirEntry> = rd.filter_map(|e| e.ok()).collect();
    entries
        .par_iter()
        .flat_map(|e| -> Vec<(PathBuf, u64)> {
            let Ok(ft) = e.file_type() else { return vec![] };
            if ft.is_symlink() {
                return vec![];
            }
            if ft.is_dir() {
                collect_files(&e.path(), c, min)
            } else if let Ok(m) = e.metadata() {
                if m.len() >= min {
                    vec![(e.path(), m.len())]
                } else {
                    vec![]
                }
            } else {
                vec![]
            }
        })
        .collect()
}

// ---------- drives and folders ----------

#[derive(Serialize)]
struct DriveInfo {
    letter: String,
    total: u64,
    free: u64,
    removable: bool,
}

#[tauri::command]
fn list_drives() -> Vec<DriveInfo> {
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives};
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{}:\\", letter);
        let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
        let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
        if kind != 2 && kind != 3 {
            continue;
        }
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free) };
        if ok == 0 || total == 0 {
            continue;
        }
        out.push(DriveInfo { letter: letter.to_string(), total, free: avail, removable: kind == 2 });
    }
    out
}

#[tauri::command]
fn user_profile() -> String {
    env_path("USERPROFILE")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "C:\\".into())
}

#[tauri::command]
fn known_folders() -> Vec<String> {
    let Some(home) = env_path("USERPROFILE") else { return vec![] };
    ["Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music"]
        .iter()
        .map(|n| home.join(n))
        .filter(|p| p.is_dir())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

#[tauri::command]
fn reveal_path(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err("That path no longer exists".into());
    }
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path))
        .spawn()
        .map_err(|e| format!("Couldn't open Explorer: {}", e))?;
    Ok(())
}

// ---------- folder scan (streams results as they finish) ----------

#[derive(Serialize, Clone)]
struct Entry {
    name: String,
    path: String,
    size: u64,
    is_dir: bool,
}

#[derive(Serialize, Clone)]
struct ScanItem {
    scan_id: u64,
    total: usize,
    entry: Entry,
}

#[tauri::command]
async fn scan_folder(app: tauri::AppHandle, path: String, scan_id: u64) -> Result<Vec<Entry>, String> {
    let c = Cancel::start(&STORAGE_GEN);
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<Entry>, String> {
        let rd = std::fs::read_dir(&path).map_err(|e| format!("Can't open {}: {}", path, e))?;
        let items: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        let total = items.len();
        let mut out: Vec<Entry> = items
            .par_iter()
            .filter_map(|p| {
                if c.cancelled() {
                    return None;
                }
                let is_dir = std::fs::symlink_metadata(p).map(|m| m.is_dir()).unwrap_or(false);
                let entry = Entry {
                    name: name_of(p),
                    path: p.to_string_lossy().into_owned(),
                    size: entry_size(p, c),
                    is_dir,
                };
                if !c.cancelled() {
                    let _ = app.emit("scan-item", ScanItem { scan_id, total, entry: entry.clone() });
                }
                Some(entry)
            })
            .collect();
        if c.cancelled() {
            return Err("cancelled".into());
        }
        out.sort_by(|a, b| b.size.cmp(&a.size));
        out.truncate(300);
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?;
    r
}

// ---------- installed apps ----------

#[derive(Serialize, Clone)]
struct AppInfo {
    name: String,
    version: String,
    publisher: String,
    size: u64,
    install_location: String,
}

struct RawApp {
    info: AppInfo,
    uninstall: String,
    est_kb: u64,
    display_icon: String,
}

fn read_registry_apps() -> Vec<RawApp> {
    let sub = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    let sub32 = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";
    let roots = [
        (RegKey::predef(HKEY_LOCAL_MACHINE), sub),
        (RegKey::predef(HKEY_LOCAL_MACHINE), sub32),
        (RegKey::predef(HKEY_CURRENT_USER), sub),
    ];
    let mut out = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for (hive, path) in roots.iter() {
        let Ok(key) = hive.open_subkey(path) else { continue };
        for name_key in key.enum_keys().filter_map(|k| k.ok()) {
            let Ok(k) = key.open_subkey(&name_key) else { continue };
            let name: String = k.get_value("DisplayName").unwrap_or_default();
            if name.trim().is_empty() {
                continue;
            }
            let system: u32 = k.get_value("SystemComponent").unwrap_or(0);
            if system == 1 {
                continue;
            }
            let parent: String = k.get_value("ParentKeyName").unwrap_or_default();
            if !parent.is_empty() {
                continue;
            }
            let uninstall: String = k.get_value("UninstallString").unwrap_or_default();
            if uninstall.trim().is_empty() {
                continue;
            }
            let version: String = k.get_value("DisplayVersion").unwrap_or_default();
            if !seen.insert((name.clone(), version.clone())) {
                continue;
            }
            let publisher: String = k.get_value("Publisher").unwrap_or_default();
            let est: u32 = k.get_value("EstimatedSize").unwrap_or(0);
            let loc: String = k.get_value("InstallLocation").unwrap_or_default();
            let display_icon: String = k.get_value("DisplayIcon").unwrap_or_default();
            out.push(RawApp {
                display_icon,
                info: AppInfo {
                    name,
                    version,
                    publisher,
                    size: 0,
                    install_location: loc.trim().trim_matches('"').to_string(),
                },
                uninstall,
                est_kb: est as u64,
            });
        }
    }
    out
}

#[tauri::command]
async fn list_apps() -> Result<Vec<AppInfo>, String> {
    let r = tauri::async_runtime::spawn_blocking(|| {
        let mut apps: Vec<AppInfo> = read_registry_apps()
            .into_par_iter()
            .map(|a| {
                let mut info = a.info;
                let est = a.est_kb * 1024;
                let real = if !info.install_location.is_empty() && Path::new(&info.install_location).exists() {
                    dir_size(Path::new(&info.install_location))
                } else {
                    0
                };
                info.size = est.max(real);
                info
            })
            .collect();
        apps.sort_by(|a, b| b.size.cmp(&a.size));
        apps
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(r)
}

// ---------- app icons ----------

fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(j) = after.find('%') {
            let var = &after[..j];
            match std::env::var(var) {
                Ok(v) => out.push_str(&v),
                Err(_) => {
                    out.push('%');
                    out.push_str(var);
                    out.push('%');
                }
            }
            rest = &after[j + 1..];
        } else {
            out.push_str(&rest[i..]);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

fn icon_source(a: &RawApp) -> Option<(PathBuf, i32)> {
    let di = a.display_icon.trim();
    if !di.is_empty() {
        let (path_part, idx) = match di.rfind(',') {
            Some(pos) => match di[pos + 1..].trim().parse::<i32>() {
                Ok(i) => (&di[..pos], i),
                Err(_) => (di, 0),
            },
            None => (di, 0),
        };
        let p = PathBuf::from(expand_env(path_part.trim().trim_matches('"')));
        if p.is_file() {
            return Some((p, idx));
        }
    }
    let loc = Path::new(&a.info.install_location);
    if !a.info.install_location.is_empty() && loc.is_dir() {
        let n = norm(&a.info.name);
        let mut exes: Vec<PathBuf> = std::fs::read_dir(loc)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().map(|x| x.eq_ignore_ascii_case("exe")).unwrap_or(false)
                    && !name_of(p).to_lowercase().starts_with("unins")
            })
            .collect();
        exes.sort_by_key(|p| !norm(&name_of(p)).contains(&n[..n.len().min(5)]));
        if let Some(p) = exes.into_iter().next() {
            return Some((p, 0));
        }
    }
    None
}

fn extract_icon_png(path: &Path, index: i32) -> Option<Vec<u8>> {
    use windows_sys::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::UI::Shell::ExtractIconExW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        let mut large: HICON = std::mem::zeroed();
        let n = ExtractIconExW(wide.as_ptr(), index, &mut large, std::ptr::null_mut(), 1);
        if n == 0 || (large as usize) == 0 {
            return None;
        }
        let mut info: ICONINFO = std::mem::zeroed();
        if GetIconInfo(large, &mut info) == 0 {
            DestroyIcon(large);
            return None;
        }
        let cleanup = |info: &ICONINFO| {
            if (info.hbmColor as usize) != 0 {
                DeleteObject(info.hbmColor as _);
            }
            if (info.hbmMask as usize) != 0 {
                DeleteObject(info.hbmMask as _);
            }
            DestroyIcon(large);
        };
        if (info.hbmColor as usize) == 0 {
            cleanup(&info);
            return None;
        }
        let mut bm: BITMAP = std::mem::zeroed();
        GetObjectW(info.hbmColor as _, std::mem::size_of::<BITMAP>() as i32, &mut bm as *mut BITMAP as *mut c_void);
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        if w <= 0 || h <= 0 || w > 256 || h > 256 {
            cleanup(&info);
            return None;
        }
        let hdc = GetDC(std::mem::zeroed());
        let make_bmi = || {
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h;
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi
        };
        let len = (w * h * 4) as usize;
        let mut color = vec![0u8; len];
        let mut mask = vec![0u8; len];
        let mut bmi = make_bmi();
        let a = GetDIBits(hdc, info.hbmColor, 0, h as u32, color.as_mut_ptr() as *mut c_void, &mut bmi, DIB_RGB_COLORS);
        let mut bmi2 = make_bmi();
        let b = if (info.hbmMask as usize) != 0 {
            GetDIBits(hdc, info.hbmMask, 0, h as u32, mask.as_mut_ptr() as *mut c_void, &mut bmi2, DIB_RGB_COLORS)
        } else {
            0
        };
        ReleaseDC(std::mem::zeroed(), hdc);
        cleanup(&info);
        if a == 0 {
            return None;
        }
        let has_alpha = color.chunks_exact(4).any(|p| p[3] != 0);
        let mut rgba = Vec::with_capacity(len);
        for (i, p) in color.chunks_exact(4).enumerate() {
            let alpha = if has_alpha {
                p[3]
            } else if b != 0 && mask[i * 4] != 0 {
                0
            } else {
                255
            };
            rgba.extend_from_slice(&[p[2], p[1], p[0], alpha]);
        }
        let mut buf = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut buf, w as u32, h as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().ok()?;
            wr.write_image_data(&rgba).ok()?;
        }
        Some(buf)
    }
}

#[tauri::command]
async fn get_app_icon(name: String, version: String) -> Result<Option<String>, String> {
    let r = tauri::async_runtime::spawn_blocking(move || -> Option<String> {
        use base64::Engine;
        let app = read_registry_apps()
            .into_iter()
            .find(|a| a.info.name == name && a.info.version == version)?;
        let (path, idx) = icon_source(&app)?;
        let png = extract_icon_png(&path, idx)?;
        Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(r)
}

#[tauri::command]
async fn uninstall_app(name: String, version: String) -> Result<String, String> {
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let app = read_registry_apps()
            .into_iter()
            .find(|a| a.info.name == name && a.info.version == version)
            .ok_or_else(|| "That app isn't registered as installed anymore".to_string())?;
        let status = std::process::Command::new("cmd")
            .raw_arg(format!("/C \"{}\"", app.uninstall))
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| format!("Couldn't start the uninstaller: {}", e))?;
        Ok(format!("Uninstaller finished (exit code {})", status.code().unwrap_or(-1)))
    })
    .await
    .map_err(|e| e.to_string())?;
    r
}

// ---------- safety rules for deleting app data ----------

const DENY_TOP: &[&str] = &[
    "microsoft", "windows", "packages", "windowsapps", "programs", "common files",
    "microsoft shared", "windows nt", "windows defender", "windowspowershell",
    "internet explorer", "windows mail", "windows media player", "modifiablewindowsapps",
    "nvidia", "nvidia corporation", "intel", "amd", "google", "mozilla", "temp", "connecteddevicesplatform",
];

fn allowed_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for k in ["LOCALAPPDATA", "APPDATA", "ProgramData", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(p) = env_path(k) {
            v.push(p);
        }
    }
    if let Some(p) = env_path("USERPROFILE") {
        v.push(p.join("Downloads"));
    }
    v
}

fn check_deletable(p: &Path) -> Result<PathBuf, String> {
    let canon = std::fs::canonicalize(p).map_err(|e| format!("{}: {}", p.display(), e))?;
    for root in allowed_roots() {
        let Ok(rc) = std::fs::canonicalize(&root) else { continue };
        if canon == rc {
            return Err(format!("{} is a protected folder", root.display()));
        }
        if let Ok(rel) = canon.strip_prefix(&rc) {
            let comps: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
                .collect();
            if comps.iter().any(|c| c == "windowsapps") {
                return Err("Protected Windows folder".into());
            }
            let is_downloads = rc.ends_with("Downloads");
            if !is_downloads && comps.len() == 1 && DENY_TOP.contains(&comps[0].as_str()) {
                return Err(format!("{} is a shared system or publisher folder", p.display()));
            }
            return Ok(canon);
        }
    }
    Err(format!("{} is outside the folders this app is allowed to clean", p.display()))
}

// ---------- leftovers ----------

#[derive(Serialize)]
struct Leftover {
    path: String,
    size: u64,
}

#[tauri::command]
async fn find_leftovers(
    name: String,
    publisher: String,
    install_location: String,
) -> Result<Vec<Leftover>, String> {
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<Leftover>, String> {
        if read_registry_apps().iter().any(|a| a.info.name == name) {
            return Err("The app is still installed, so leftovers weren't scanned.".into());
        }
        let n = norm(&name);
        let pubn = norm(publisher.split(|c: char| !c.is_alphanumeric()).next().unwrap_or(""));
        let short = if !pubn.is_empty() && n.starts_with(&pubn) && n.len() > pubn.len() {
            n[pubn.len()..].to_string()
        } else {
            n.clone()
        };
        if short.len() < 4 {
            return Err("The app name is too short to match leftovers safely.".into());
        }
        let matches = |s: &str| {
            let x = norm(s);
            x == n || x.contains(&short)
        };

        let mut found: Vec<PathBuf> = Vec::new();
        for root in allowed_roots() {
            if root.ends_with("Downloads") {
                continue;
            }
            let Ok(rd) = std::fs::read_dir(&root) else { continue };
            for child in rd.filter_map(|e| e.ok()).map(|e| e.path()) {
                if !child.is_dir() {
                    continue;
                }
                if matches(&name_of(&child)) {
                    found.push(child);
                } else if let Ok(rd2) = std::fs::read_dir(&child) {
                    for g in rd2.filter_map(|e| e.ok()).map(|e| e.path()) {
                        if g.is_dir() && matches(&name_of(&g)) {
                            found.push(g);
                        }
                    }
                }
            }
        }
        if !install_location.is_empty() {
            let p = PathBuf::from(&install_location);
            if p.exists() && !found.iter().any(|f| p.starts_with(f) || f.starts_with(&p)) {
                found.push(p);
            }
        }
        let mut out: Vec<Leftover> = found
            .into_iter()
            .filter(|p| check_deletable(p).is_ok())
            .collect::<Vec<_>>()
            .par_iter()
            .map(|p| Leftover { path: p.to_string_lossy().into_owned(), size: dir_size(p) })
            .collect();
        out.sort_by(|a, b| b.size.cmp(&a.size));
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?;
    r
}

#[derive(Serialize)]
struct DeleteResult {
    path: String,
    ok: bool,
    error: Option<String>,
    freed: u64,
}

#[tauri::command]
async fn delete_paths(paths: Vec<String>, permanent: bool) -> Result<Vec<DeleteResult>, String> {
    let r = tauri::async_runtime::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|s| {
                let p = PathBuf::from(&s);
                let canon = match check_deletable(&p) {
                    Ok(c) => c,
                    Err(e) => return DeleteResult { path: s, ok: false, error: Some(e), freed: 0 },
                };
                let size = dir_size(&p);
                let res: Result<(), String> = if permanent {
                    if canon.is_dir() {
                        std::fs::remove_dir_all(&p).map_err(|e| e.to_string())
                    } else {
                        std::fs::remove_file(&p).map_err(|e| e.to_string())
                    }
                } else {
                    trash::delete(&p).map_err(|e| e.to_string())
                };
                match res {
                    Ok(()) => DeleteResult { path: s, ok: true, error: None, freed: size },
                    Err(e) => DeleteResult { path: s, ok: false, error: Some(e), freed: 0 },
                }
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(r)
}

// ---------- large files and duplicates ----------

#[derive(Serialize)]
struct FileHit {
    path: String,
    name: String,
    size: u64,
}

#[derive(Serialize)]
struct DupGroup {
    size: u64,
    files: Vec<String>,
}

fn collect_from_roots(roots: &[String], c: Cancel, min: u64) -> Vec<(PathBuf, u64)> {
    let mut all = Vec::new();
    for r in roots {
        let p = PathBuf::from(r);
        if p.is_dir() {
            all.extend(collect_files(&p, c, min));
        }
    }
    all
}

#[tauri::command]
async fn find_large_files(app: tauri::AppHandle, roots: Vec<String>, min_mb: u64) -> Result<Vec<FileHit>, String> {
    let c = Cancel::start(&FILES_GEN);
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<FileHit>, String> {
        let _ = app.emit("files-progress", "Scanning for large filesâ€¦");
        let mut all = collect_from_roots(&roots, c, min_mb.max(1) * 1024 * 1024);
        if c.cancelled() {
            return Err("cancelled".into());
        }
        all.sort_by(|a, b| b.1.cmp(&a.1));
        all.truncate(200);
        Ok(all
            .into_iter()
            .map(|(p, s)| FileHit { name: name_of(&p), path: p.to_string_lossy().into_owned(), size: s })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?;
    r
}

fn hash_file(p: &Path, limit: Option<usize>) -> Option<Vec<u8>> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut left = limit.unwrap_or(usize::MAX);
    while left > 0 {
        let want = buf.len().min(left);
        let n = f.read(&mut buf[..want]).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        left -= n;
    }
    Some(h.finalize().to_vec())
}

fn group_by_hash(items: Vec<(u64, Vec<u8>, PathBuf)>) -> Vec<Vec<(u64, PathBuf)>> {
    let mut map: HashMap<(u64, Vec<u8>), Vec<(u64, PathBuf)>> = HashMap::new();
    for (s, h, p) in items {
        map.entry((s, h)).or_default().push((s, p));
    }
    map.into_values().filter(|v| v.len() > 1).collect()
}

#[tauri::command]
async fn find_duplicates(app: tauri::AppHandle, roots: Vec<String>, min_mb: u64) -> Result<Vec<DupGroup>, String> {
    let c = Cancel::start(&FILES_GEN);
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<DupGroup>, String> {
        let _ = app.emit("files-progress", "Listing filesâ€¦");
        let mut files = collect_from_roots(&roots, c, min_mb.max(1) * 1024 * 1024);
        files.retain(|(p, _)| !p.to_string_lossy().to_lowercase().contains("\\appdata\\"));
        if c.cancelled() {
            return Err("cancelled".into());
        }
        let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
        for (p, s) in files {
            by_size.entry(s).or_default().push(p);
        }
        let cands: Vec<(u64, PathBuf)> = by_size
            .into_iter()
            .filter(|(_, v)| v.len() > 1)
            .flat_map(|(s, v)| v.into_iter().map(move |p| (s, p)))
            .collect();
        let _ = app.emit("files-progress", format!("Comparing {} files of equal sizeâ€¦", cands.len()));

        let stage1: Vec<(u64, Vec<u8>, PathBuf)> = cands
            .par_iter()
            .filter_map(|(s, p)| {
                if c.cancelled() {
                    return None;
                }
                hash_file(p, Some(64 * 1024)).map(|h| (*s, h, p.clone()))
            })
            .collect();
        let narrowed: Vec<(u64, PathBuf)> = group_by_hash(stage1).into_iter().flatten().collect();
        let _ = app.emit("files-progress", format!("Confirming {} likely duplicatesâ€¦", narrowed.len()));

        let stage2: Vec<(u64, Vec<u8>, PathBuf)> = narrowed
            .par_iter()
            .filter_map(|(s, p)| {
                if c.cancelled() {
                    return None;
                }
                hash_file(p, None).map(|h| (*s, h, p.clone()))
            })
            .collect();
        if c.cancelled() {
            return Err("cancelled".into());
        }
        let mut groups: Vec<DupGroup> = group_by_hash(stage2)
            .into_iter()
            .map(|v| {
                let size = v[0].0;
                let mut files: Vec<String> = v.into_iter().map(|(_, p)| p.to_string_lossy().into_owned()).collect();
                files.sort();
                DupGroup { size, files }
            })
            .collect();
        groups.sort_by(|a, b| {
            let wa = a.size * (a.files.len() as u64 - 1);
            let wb = b.size * (b.files.len() as u64 - 1);
            wb.cmp(&wa)
        });
        groups.truncate(150);
        Ok(groups)
    })
    .await
    .map_err(|e| e.to_string())?;
    r
}

#[tauri::command]
fn cancel_files_scan() {
    FILES_GEN.fetch_add(1, Ordering::SeqCst);
}

fn check_trashable_file(p: &Path) -> Result<PathBuf, String> {
    let canon = std::fs::canonicalize(p).map_err(|e| format!("{}: {}", p.display(), e))?;
    let md = std::fs::metadata(&canon).map_err(|e| e.to_string())?;
    if !md.is_file() {
        return Err("Only files can be removed from here".into());
    }
    let lower = canon.to_string_lossy().to_lowercase();
    let s = lower.trim_start_matches(r"\\?\").to_string();
    let mut blocked: Vec<String> = Vec::new();
    for k in ["SystemRoot", "ProgramFiles", "ProgramFiles(x86)", "ProgramData", "LOCALAPPDATA", "APPDATA"] {
        if let Some(v) = env_path(k) {
            blocked.push(v.to_string_lossy().to_lowercase());
        }
    }
    if blocked.iter().any(|b| s.starts_with(b.as_str())) {
        return Err(format!("{} belongs to Windows or an installed app. Remove it by uninstalling the app instead.", p.display()));
    }
    let after_drive = s.get(2..).unwrap_or("");
    if after_drive.starts_with("\\windows\\") || after_drive.starts_with("\\program files") {
        return Err(format!("{} is in a protected system folder", p.display()));
    }
    Ok(canon)
}

/// Moves files to the Recycle Bin only (recoverable).
#[tauri::command]
async fn trash_files(paths: Vec<String>) -> Result<Vec<DeleteResult>, String> {
    let r = tauri::async_runtime::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|s| {
                let p = PathBuf::from(&s);
                if let Err(e) = check_trashable_file(&p) {
                    return DeleteResult { path: s, ok: false, error: Some(e), freed: 0 };
                }
                let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                match trash::delete(&p) {
                    Ok(()) => DeleteResult { path: s, ok: true, error: None, freed: size },
                    Err(e) => DeleteResult { path: s, ok: false, error: Some(e.to_string()), freed: 0 },
                }
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(r)
}

// ---------- caches ----------

#[derive(Clone, Copy, PartialEq)]
enum CacheKind {
    Folder,
    RecycleBin,
}

struct CacheDef {
    id: &'static str,
    label: &'static str,
    note: &'static str,
    paths: Vec<PathBuf>,
    prefix: Option<&'static str>,
    default_on: bool,
    kind: CacheKind,
}

fn folder_def(id: &'static str, label: &'static str, note: &'static str, paths: Vec<PathBuf>, default_on: bool) -> CacheDef {
    CacheDef { id, label, note, paths, prefix: None, default_on, kind: CacheKind::Folder }
}

fn browser_cache_paths(user_data: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir(user_data) else { return v };
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        let n = name_of(&p);
        if p.is_dir() && (n == "Default" || n.starts_with("Profile ")) {
            for sub in ["Cache", "Code Cache", "GPUCache"] {
                v.push(p.join(sub));
            }
        }
    }
    v
}

fn firefox_cache_paths(local: &Path) -> Vec<PathBuf> {
    let profiles = local.join("Mozilla").join("Firefox").join("Profiles");
    let Ok(rd) = std::fs::read_dir(&profiles) else { return vec![] };
    rd.filter_map(|e| e.ok())
        .map(|e| e.path().join("cache2"))
        .collect()
}

fn cache_defs() -> Vec<CacheDef> {
    let local = env_path("LOCALAPPDATA").unwrap_or_default();
    let roaming = env_path("APPDATA").unwrap_or_default();
    let temp = env_path("TEMP").unwrap_or_default();
    let win = env_path("SystemRoot").unwrap_or_else(|| PathBuf::from("C:\\Windows"));
    let chrome = local.join("Google").join("Chrome").join("User Data");
    let edge = local.join("Microsoft").join("Edge").join("User Data");
    let brave = local.join("BraveSoftware").join("Brave-Browser").join("User Data");
    let win_explorer = local.join("Microsoft").join("Windows").join("Explorer");

    vec![
        folder_def("temp", "Temporary files", "Your user temp folder. Apps recreate files as needed", vec![temp], true),
        folder_def("wintemp", "Windows temp files", "Some items need administrator rights and are skipped", vec![win.join("Temp")], true),
        CacheDef {
            id: "recycle",
            label: "Recycle Bin",
            note: "Permanently empties the Recycle Bin on every drive",
            paths: vec![],
            prefix: None,
            default_on: false,
            kind: CacheKind::RecycleBin,
        },
        CacheDef {
            id: "thumbs",
            label: "Thumbnail cache",
            note: "Explorer rebuilds picture previews when you open folders",
            paths: vec![win_explorer],
            prefix: Some("thumbcache_"),
            default_on: true,
            kind: CacheKind::Folder,
        },
        folder_def("d3d", "DirectX shader cache", "Games and apps rebuild it on next launch", vec![local.join("D3DSCache")], true),
        folder_def(
            "nvidia",
            "NVIDIA shader cache",
            "Games rebuild it on next launch",
            vec![local.join("NVIDIA").join("DXCache"), local.join("NVIDIA").join("GLCache")],
            true,
        ),
        folder_def("npm", "npm cache", "Packages download again when needed", vec![local.join("npm-cache")], true),
        folder_def("pip", "pip cache", "Python packages download again when needed", vec![local.join("pip").join("Cache")], true),
        folder_def("wer", "Windows error reports", "Crash reports queued for Microsoft", vec![local.join("Microsoft").join("Windows").join("WER")], true),
        folder_def("crash", "Crash dumps", "Only useful for debugging crashes", vec![local.join("CrashDumps")], true),
        folder_def("chrome", "Chrome cache", "Close Chrome first. Sign-ins and bookmarks stay; pages load slower once", browser_cache_paths(&chrome), false),
        folder_def("chromeai", "Chrome on-device AI model", "Chrome downloads it again if it needs it", vec![chrome.join("OptGuideOnDeviceModel")], false),
        folder_def("edge", "Edge cache", "Close Edge first. Sign-ins and bookmarks stay", browser_cache_paths(&edge), false),
        folder_def("brave", "Brave cache", "Close Brave first. Sign-ins and bookmarks stay", browser_cache_paths(&brave), false),
        folder_def("firefox", "Firefox cache", "Close Firefox first. Sign-ins and bookmarks stay", firefox_cache_paths(&local), false),
        folder_def(
            "discord",
            "Discord cache",
            "Close Discord first. Images reload when needed",
            vec![roaming.join("discord").join("Cache"), roaming.join("discord").join("Code Cache"), roaming.join("discord").join("GPUCache")],
            false,
        ),
        folder_def(
            "vscode",
            "VS Code cache",
            "Close VS Code first. It rebuilds the cache on next start",
            vec![
                roaming.join("Code").join("Cache"),
                roaming.join("Code").join("CachedData"),
                roaming.join("Code").join("Code Cache"),
                roaming.join("Code").join("GPUCache"),
            ],
            false,
        ),
    ]
}

fn cache_entries(d: &CacheDef) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for dir in d.paths.iter().filter(|p| p.is_dir()) {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path();
            if let Some(pre) = d.prefix {
                if !name_of(&p).to_lowercase().starts_with(pre) {
                    continue;
                }
            }
            v.push(p);
        }
    }
    v
}

fn recycle_bin_size() -> u64 {
    use windows_sys::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};
    unsafe {
        let mut info: SHQUERYRBINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHQUERYRBINFO>() as u32;
        let hr = SHQueryRecycleBinW(std::ptr::null(), &mut info);
        if hr != 0 {
            return 0;
        }
        let sz = info.i64Size;
        sz.max(0) as u64
    }
}

fn empty_recycle_bin() -> bool {
    use windows_sys::Win32::UI::Shell::SHEmptyRecycleBinW;
    // SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND
    unsafe { SHEmptyRecycleBinW(std::mem::zeroed(), std::ptr::null(), 0x7) == 0 }
}

#[derive(Serialize)]
struct CacheTarget {
    id: String,
    label: String,
    note: String,
    size: u64,
    default_on: bool,
}

#[tauri::command]
async fn cache_targets() -> Result<Vec<CacheTarget>, String> {
    let r = tauri::async_runtime::spawn_blocking(|| {
        let mut v: Vec<CacheTarget> = cache_defs()
            .into_iter()
            .map(|d| {
                let size = match d.kind {
                    CacheKind::RecycleBin => recycle_bin_size(),
                    CacheKind::Folder => cache_entries(&d).par_iter().map(|p| dir_size(p)).sum(),
                };
                CacheTarget { id: d.id.into(), label: d.label.into(), note: d.note.into(), size, default_on: d.default_on }
            })
            .filter(|t| t.size > 0)
            .collect();
        v.sort_by(|a, b| b.size.cmp(&a.size));
        v
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(r)
}

#[derive(Serialize)]
struct CleanReport {
    freed: u64,
    skipped: usize,
}

#[tauri::command]
async fn clean_cache(ids: Vec<String>) -> Result<CleanReport, String> {
    let r = tauri::async_runtime::spawn_blocking(move || {
        let mut freed = 0u64;
        let mut skipped = 0usize;
        for def in cache_defs().into_iter().filter(|d| ids.iter().any(|i| i == d.id)) {
            if def.kind == CacheKind::RecycleBin {
                let size = recycle_bin_size();
                if empty_recycle_bin() {
                    freed += size;
                } else {
                    skipped += 1;
                }
                continue;
            }
            for child in cache_entries(&def) {
                let size = dir_size(&child);
                let res = if child.is_dir() { std::fs::remove_dir_all(&child) } else { std::fs::remove_file(&child) };
                match res {
                    Ok(()) => freed += size,
                    Err(_) => skipped += 1,
                }
            }
        }
        CleanReport { freed, skipped }
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(r)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            list_drives,
            user_profile,
            known_folders,
            reveal_path,
            scan_folder,
            list_apps,
            get_app_icon,
            uninstall_app,
            find_leftovers,
            delete_paths,
            find_large_files,
            find_duplicates,
            cancel_files_scan,
            trash_files,
            cache_targets,
            clean_cache
        ])
        .run(tauri::generate_context!())
        .expect("error while running Dustoff");
}

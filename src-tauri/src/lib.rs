use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::io::Read;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
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
    installed: String,
    installed_approx: bool,
    last_used: String,
    /// The uninstaller file is gone, so Windows still lists the app but it can't be uninstalled.
    broken: bool,
}

struct RawApp {
    info: AppInfo,
    uninstall: String,
    est_kb: u64,
    display_icon: String,
    /// 0 = HKLM, 1 = HKLM (32-bit view), 2 = HKCU
    hive: u8,
    key_name: String,
}

const UNINSTALL_KEYS: [&str; 3] = [
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
];

fn uninstall_exe(s: &str) -> Option<PathBuf> {
    let s = s.trim();
    if s.to_lowercase().starts_with("msiexec") {
        return None;
    }
    if let Some(rest) = s.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(PathBuf::from(&rest[..end]));
    }
    let lower = s.to_lowercase();
    lower.find(".exe").map(|i| PathBuf::from(&s[..i + 4]))
}

/// True only when the uninstall command points at an absolute path that no longer exists.
fn is_broken(uninstall: &str) -> bool {
    match uninstall_exe(&expand_env(uninstall)) {
        Some(p) => p.is_absolute() && !p.exists(),
        None => false,
    }
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
    for (hive_idx, (hive, path)) in roots.iter().enumerate() {
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
            let inst_raw: String = k.get_value("InstallDate").unwrap_or_else(|_| {
                k.get_value::<u32, _>("InstallDate").map(|v| v.to_string()).unwrap_or_default()
            });
            let mut installed = if inst_raw.len() == 8 && inst_raw.chars().all(|c| c.is_ascii_digit()) {
                format!("{}-{}-{}", &inst_raw[0..4], &inst_raw[4..6], &inst_raw[6..8])
            } else {
                String::new()
            };
            let mut installed_approx = false;
            if installed.is_empty() {
                if let Ok(meta) = k.query_info() {
                    let t = meta.get_last_write_time_system();
                    if t.wYear > 1990 {
                        installed = format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay);
                        installed_approx = true;
                    }
                }
            }
            let display_icon: String = k.get_value("DisplayIcon").unwrap_or_default();
            out.push(RawApp {
                display_icon,
                hive: hive_idx as u8,
                key_name: name_key.clone(),
                info: AppInfo {
                    name,
                    version,
                    publisher,
                    size: 0,
                    installed,
                    installed_approx,
                    last_used: String::new(),
                    broken: is_broken(&uninstall),
                    install_location: loc.trim().trim_matches('"').to_string(),
                },
                uninstall,
                est_kb: est as u64,
            });
        }
    }
    out
}

// Windows records when programs were launched from Start or Explorer (UserAssist, ROT13-encoded names).
fn rot13(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char,
            'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char,
            _ => c,
        })
        .collect()
}

fn unix_to_date(secs: u64) -> String {
    let z = (secs / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 {
        y += 1;
    }
    format!("{:04}-{:02}-{:02}", y, m, d)
}

fn userassist_entries() -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let hk = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(root) = hk.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist") else { return out };
    for g in root.enum_keys().filter_map(|k| k.ok()) {
        let Ok(cnt) = root.open_subkey(format!("{}\\Count", g)) else { continue };
        for (name, val) in cnt.enum_values().filter_map(|v| v.ok()) {
            let b: &[u8] = &val.bytes;
            if b.len() < 68 {
                continue;
            }
            let ft = u64::from_le_bytes(b[60..68].try_into().unwrap_or([0u8; 8]));
            let secs = ft / 10_000_000;
            if secs <= 11_644_473_600 {
                continue;
            }
            out.push((rot13(&name).to_lowercase(), secs - 11_644_473_600));
        }
    }
    out
}

/// App name without a trailing version, normalised (for example "Docker Desktop 4.2.1" becomes "dockerdesktop").
fn app_key(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let mut end = words.len();
    while end > 1
        && words[end - 1].chars().any(|c| c.is_ascii_digit())
        && words[end - 1].chars().all(|c| c.is_ascii_digit() || c == '.' || c == 'v' || c == 'V')
    {
        end -= 1;
    }
    norm(&words[..end].join(" "))
}

fn last_used_for(app: &RawApp, ua: &[(String, u64)]) -> String {
    // 1) launches of the program file itself
    let by_exe = icon_source(app).and_then(|(exe, _)| {
        if !exe.extension().map(|x| x.eq_ignore_ascii_case("exe")).unwrap_or(false) {
            return None;
        }
        let fname = name_of(&exe).to_lowercase();
        let suffix = format!("\\{}", fname);
        let hint = Path::new(&app.info.install_location)
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        ua.iter()
            .filter(|(p, _)| p.ends_with(&suffix) || *p == fname)
            .filter(|(p, _)| hint.is_empty() || p.contains(hint.as_str()))
            .map(|(_, t)| *t)
            .max()
    });
    // 2) launches through a Start Menu shortcut named like the app
    let key = app_key(&app.info.name);
    let by_name = if key.len() >= 5 {
        ua.iter()
            .filter(|(p, _)| {
                let stem = Path::new(p)
                    .file_stem()
                    .map(|s| norm(&s.to_string_lossy()))
                    .unwrap_or_default();
                stem.len() >= 5 && (stem == key || stem.contains(&key) || (stem.len() >= 8 && key.contains(&stem)))
            })
            .map(|(_, t)| *t)
            .max()
    } else {
        None
    };
    by_exe
        .into_iter()
        .chain(by_name)
        .max()
        .map(unix_to_date)
        .unwrap_or_default()
}
#[tauri::command]
async fn list_apps() -> Result<Vec<AppInfo>, String> {
    let r = tauri::async_runtime::spawn_blocking(|| {
        let ua = userassist_entries();
        let mut apps: Vec<AppInfo> = read_registry_apps()
            .into_par_iter()
            .map(|a| {
                let used = last_used_for(&a, &ua);
                let mut info = a.info;
                info.last_used = used;
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
        if is_broken(&app.uninstall) {
            return Err("This app's uninstaller is missing, so it can't be uninstalled. Remove the broken entry instead.".into());
        }
        let status = std::process::Command::new("cmd")
            .raw_arg(format!("/C \"{}\"", app.uninstall))
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| format!("Couldn't start the uninstaller: {}", e))?;
        let gone = !read_registry_apps().iter().any(|a| a.info.name == name);
        if gone {
            record_history("uninstall", format!("Uninstalled {}", name), 0, vec![]);
        }
        Ok(format!("Uninstaller finished (exit code {})", status.code().unwrap_or(-1)))
    })
    .await
    .map_err(|e| e.to_string())?;
    r
}

fn registry_error(e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        "Windows needs administrator rights to remove this entry. Close Dustoff, right-click it, choose Run as administrator, and try again.".to_string()
    } else {
        format!("Couldn't change the registry: {}", e)
    }
}

/// Removes the entry of an app whose uninstaller no longer exists (a "ghost" in the installed apps list).
#[tauri::command]
async fn remove_broken_entry(name: String, version: String) -> Result<(), String> {
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let app = read_registry_apps()
            .into_iter()
            .find(|a| a.info.name == name && a.info.version == version)
            .ok_or_else(|| "That app isn't registered as installed anymore".to_string())?;
        if !is_broken(&app.uninstall) {
            return Err("This app's uninstaller still exists, so use Uninstall instead.".into());
        }
        let hk = if app.hive == 2 { RegKey::predef(HKEY_CURRENT_USER) } else { RegKey::predef(HKEY_LOCAL_MACHINE) };
        let parent = hk
            .open_subkey_with_flags(UNINSTALL_KEYS[app.hive as usize], KEY_ALL_ACCESS)
            .map_err(registry_error)?;
        parent.delete_subkey_all(&app.key_name).map_err(registry_error)?;
        record_history("uninstall", format!("Removed broken entry for {}", name), 0, vec![]);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?;
    r
}

// ---------- saved data: protected folders and activity log ----------

static STORE_LOCK: Mutex<()> = Mutex::new(());

fn data_dir() -> PathBuf {
    let base = env_path("APPDATA").unwrap_or_else(std::env::temp_dir);
    let d = base.join("Dustoff");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn norm_path(p: &Path) -> String {
    let raw = p.to_string_lossy().to_string();
    let mut s = raw.trim_start_matches(r"\\?\").replace('/', "\\").to_lowercase();
    while s.ends_with('\\') {
        s.pop();
    }
    s
}

fn load_protected() -> Vec<String> {
    std::fs::read_to_string(data_dir().join("protected.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_protected(v: &Vec<String>) {
    let _ = std::fs::write(
        data_dir().join("protected.json"),
        serde_json::to_string_pretty(v).unwrap_or_default(),
    );
}

/// Returns the protected folder that conflicts with `p` (p is inside it, or p contains it).
fn conflict_with(p: &Path, list: &[String]) -> Option<String> {
    let target = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let t = format!("{}\\", norm_path(&target));
    for pr in list {
        let prp = PathBuf::from(pr);
        let c = std::fs::canonicalize(&prp).unwrap_or(prp);
        let q = format!("{}\\", norm_path(&c));
        if t.starts_with(&q) || q.starts_with(&t) {
            return Some(pr.clone());
        }
    }
    None
}

fn protected_conflict(p: &Path) -> Option<String> {
    conflict_with(p, &load_protected())
}

/// Shows a Windows notification (toast). Returns the real error if Windows refuses it.
#[tauri::command]
fn send_notification(app: tauri::AppHandle, title: String, body: String) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| format!("Windows didn't show the notification: {}", e))
}

#[tauri::command]
fn settings_load() -> String {
    std::fs::read_to_string(data_dir().join("settings.json")).unwrap_or_else(|_| "{}".into())
}

#[tauri::command]
fn settings_save(json: String) -> Result<(), String> {
    serde_json::from_str::<serde_json::Value>(&json).map_err(|e| format!("Invalid settings: {}", e))?;
    let _g = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::fs::write(data_dir().join("settings.json"), json).map_err(|e| format!("Couldn't save settings: {}", e))
}

#[tauri::command]
fn data_folder() -> String {
    data_dir().to_string_lossy().into_owned()
}

#[tauri::command]
fn open_data_folder() -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg(data_dir())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't open File Explorer: {}", e))
}

#[tauri::command]
fn protected_list() -> Vec<String> {
    load_protected()
}

#[tauri::command]
fn protect_add(path: String) -> Result<Vec<String>, String> {
    let _g = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let p = PathBuf::from(path.trim());
    if !p.is_dir() {
        return Err("That folder doesn't exist".into());
    }
    let canon = std::fs::canonicalize(&p).map_err(|e| e.to_string())?;
    let s = canon.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    let mut v = load_protected();
    if !v.iter().any(|x| norm_path(Path::new(x)) == norm_path(Path::new(&s))) {
        v.push(s);
        v.sort();
        save_protected(&v);
    }
    Ok(v)
}

#[tauri::command]
fn protect_remove(path: String) -> Vec<String> {
    let _g = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut v = load_protected();
    let want = norm_path(Path::new(&path));
    v.retain(|x| norm_path(Path::new(x)) != want);
    save_protected(&v);
    v
}

#[derive(Serialize, Deserialize, Clone)]
struct HistItem {
    path: String,
    size: u64,
    recycled: bool,
}

#[derive(Serialize, Deserialize, Clone)]
struct HistEntry {
    id: u64,
    ts: u64,
    kind: String,
    title: String,
    freed: u64,
    items: Vec<HistItem>,
    undone: bool,
}

fn load_history() -> Vec<HistEntry> {
    std::fs::read_to_string(data_dir().join("history.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_history(v: &Vec<HistEntry>) {
    let _ = std::fs::write(
        data_dir().join("history.json"),
        serde_json::to_string(v).unwrap_or_default(),
    );
}

fn record_history(kind: &str, title: String, freed: u64, mut items: Vec<HistItem>) {
    let _g = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = load_history();
    items.truncate(500);
    h.push(HistEntry { id: now_millis(), ts: now_secs(), kind: kind.into(), title, freed, items, undone: false });
    if h.len() > 300 {
        let n = h.len() - 300;
        h.drain(0..n);
    }
    save_history(&h);
}

#[tauri::command]
fn list_history() -> Vec<HistEntry> {
    let mut h = load_history();
    h.reverse();
    h
}

#[tauri::command]
fn clear_history() {
    let _g = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    save_history(&Vec::new());
}

#[derive(Serialize)]
struct UndoFail {
    path: String,
    error: String,
}

#[derive(Serialize)]
struct UndoReport {
    restored: usize,
    failed: Vec<UndoFail>,
}

#[tauri::command]
async fn undo_entry(id: u64) -> Result<UndoReport, String> {
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<UndoReport, String> {
        use trash::os_limited::{list, restore_all};
        let entry = load_history()
            .into_iter()
            .find(|e| e.id == id)
            .ok_or_else(|| "That entry no longer exists".to_string())?;
        if entry.undone {
            return Err("This was already restored".into());
        }
        let bin = list().map_err(|e| format!("Couldn't read the Recycle Bin: {}", e))?;
        let mut restored = 0usize;
        let mut failed: Vec<UndoFail> = Vec::new();
        for it in entry.items.iter().filter(|i| i.recycled) {
            let want = norm_path(Path::new(&it.path));
            let mut cands: Vec<&trash::TrashItem> = bin
                .iter()
                .filter(|ti| norm_path(&ti.original_parent.join(&ti.name)) == want)
                .collect();
            cands.sort_by_key(|ti| std::cmp::Reverse(ti.time_deleted));
            match cands.first() {
                None => failed.push(UndoFail {
                    path: it.path.clone(),
                    error: "It's no longer in the Recycle Bin (emptied or already restored)".into(),
                }),
                Some(ti) => match restore_all(vec![(*ti).clone()]) {
                    Ok(()) => restored += 1,
                    Err(e) => failed.push(UndoFail { path: it.path.clone(), error: format!("{}", e) }),
                },
            }
        }
        if failed.is_empty() && restored > 0 {
            let _g = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut h = load_history();
            if let Some(e) = h.iter_mut().find(|e| e.id == id) {
                e.undone = true;
            }
            save_history(&h);
        }
        Ok(UndoReport { restored, failed })
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
    if let Some(pr) = protected_conflict(p) {
        return Err(format!("{} is inside a folder you protected ({})", p.display(), pr));
    }
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
        // Start Menu shortcuts and the publisher folder that Squirrel/Electron installers leave behind.
        if let Some(app_data) = env_path("APPDATA") {
            let programs = app_data.join("Microsoft").join("Windows").join("Start Menu").join("Programs");
            let stem_of = |p: &Path| p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let publisher_norm = norm(&publisher);
            if let Ok(rd) = std::fs::read_dir(&programs) {
                for child in rd.filter_map(|e| e.ok()).map(|e| e.path()) {
                    let nm = name_of(&child);
                    if child.is_file() {
                        let is_lnk = child.extension().map(|x| x.eq_ignore_ascii_case("lnk")).unwrap_or(false);
                        if is_lnk && matches(&stem_of(&child)) {
                            found.push(child);
                        }
                    } else if child.is_dir() {
                        if matches(&nm) {
                            found.push(child);
                        } else if !publisher_norm.is_empty() && norm(&nm) == publisher_norm {
                            // a publisher folder is removed only when it holds nothing but this app's shortcuts
                            let only_ours = std::fs::read_dir(&child)
                                .map(|r| r.filter_map(|e| e.ok()).map(|e| e.path()).all(|p| p.is_file() && matches(&stem_of(&p))))
                                .unwrap_or(false);
                            if only_ours {
                                found.push(child);
                            }
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

#[derive(Serialize, Clone)]
struct Progress {
    label: String,
    done: usize,
    total: usize,
    freed: u64,
}

fn emit_progress(app: &tauri::AppHandle, label: &str, done: usize, total: usize, freed: u64) {
    let _ = app.emit("task-progress", Progress { label: label.to_string(), done, total, freed });
}

#[tauri::command]
async fn delete_paths(
    app: tauri::AppHandle,
    paths: Vec<String>,
    permanent: bool,
    title: Option<String>,
) -> Result<Vec<DeleteResult>, String> {
    let r = tauri::async_runtime::spawn_blocking(move || {
        let total = paths.len();
        let mut results: Vec<DeleteResult> = Vec::new();
        let mut freed_total = 0u64;
        for (i, s) in paths.into_iter().enumerate() {
            emit_progress(&app, &s, i, total, freed_total);
            let p = PathBuf::from(&s);
            let res = match check_deletable(&p) {
                Err(e) => DeleteResult { path: s, ok: false, error: Some(e), freed: 0 },
                Ok(canon) => {
                    let size = dir_size(&p);
                    let outcome: Result<(), String> = if permanent {
                        if canon.is_dir() {
                            std::fs::remove_dir_all(&p).map_err(|e| e.to_string())
                        } else {
                            std::fs::remove_file(&p).map_err(|e| e.to_string())
                        }
                    } else {
                        trash::delete(&p).map_err(|e| e.to_string())
                    };
                    match outcome {
                        Ok(()) => DeleteResult { path: s, ok: true, error: None, freed: size },
                        Err(e) => DeleteResult { path: s, ok: false, error: Some(e), freed: 0 },
                    }
                }
            };
            freed_total += res.freed;
            results.push(res);
        }
        emit_progress(&app, "Done", total, total, freed_total);
        let items: Vec<HistItem> = results
            .iter()
            .filter(|r| r.ok)
            .map(|r| HistItem { path: r.path.clone(), size: r.freed, recycled: !permanent })
            .collect();
        if !items.is_empty() {
            record_history("remove", title.unwrap_or_else(|| "Removed folders".into()), freed_total, items);
        }
        results
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
    if let Some(pr) = protected_conflict(p) {
        return Err(format!("{} is inside a folder you protected ({})", p.display(), pr));
    }
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
async fn trash_files(app: tauri::AppHandle, paths: Vec<String>, title: Option<String>) -> Result<Vec<DeleteResult>, String> {
    let r = tauri::async_runtime::spawn_blocking(move || {
        let total = paths.len();
        let mut results: Vec<DeleteResult> = Vec::new();
        let mut freed_total = 0u64;
        for (i, s) in paths.into_iter().enumerate() {
            emit_progress(&app, &s, i, total, freed_total);
            let p = PathBuf::from(&s);
            let res = if let Err(e) = check_trashable_file(&p) {
                DeleteResult { path: s, ok: false, error: Some(e), freed: 0 }
            } else {
                let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                match trash::delete(&p) {
                    Ok(()) => DeleteResult { path: s, ok: true, error: None, freed: size },
                    Err(e) => DeleteResult { path: s, ok: false, error: Some(e.to_string()), freed: 0 },
                }
            };
            freed_total += res.freed;
            results.push(res);
        }
        emit_progress(&app, "Done", total, total, freed_total);
        let items: Vec<HistItem> = results
            .iter()
            .filter(|r| r.ok)
            .map(|r| HistItem { path: r.path.clone(), size: r.freed, recycled: true })
            .collect();
        if !items.is_empty() {
            record_history("files", title.unwrap_or_else(|| "Files moved to the Recycle Bin".into()), freed_total, items);
        }
        results
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
struct CleanDetail {
    id: String,
    label: String,
    freed: u64,
    skipped: usize,
}

#[derive(Serialize)]
struct CleanReport {
    freed: u64,
    skipped: usize,
    details: Vec<CleanDetail>,
}

#[tauri::command]
async fn clean_cache(app: tauri::AppHandle, ids: Vec<String>, title: Option<String>) -> Result<CleanReport, String> {
    let r = tauri::async_runtime::spawn_blocking(move || {
        let protected = load_protected();
        let plans: Vec<(CacheDef, Vec<PathBuf>)> = cache_defs()
            .into_iter()
            .filter(|d| ids.iter().any(|i| i == d.id))
            .map(|d| {
                let entries = if d.kind == CacheKind::RecycleBin { Vec::new() } else { cache_entries(&d) };
                (d, entries)
            })
            .collect();
        let total: usize = plans
            .iter()
            .map(|(d, e)| if d.kind == CacheKind::RecycleBin { 1 } else { e.len().max(1) })
            .sum();
        let mut done = 0usize;
        let mut freed = 0u64;
        let mut skipped = 0usize;
        let mut details: Vec<CleanDetail> = Vec::new();
        let mut last = Instant::now() - Duration::from_secs(1);

        for (def, entries) in plans {
            let mut d_freed = 0u64;
            let mut d_skipped = 0usize;
            emit_progress(&app, def.label, done, total, freed);
            if def.kind == CacheKind::RecycleBin {
                let size = recycle_bin_size();
                if empty_recycle_bin() {
                    d_freed = size;
                } else {
                    d_skipped = 1;
                }
                done += 1;
            } else {
                if entries.is_empty() {
                    done += 1;
                }
                for child in entries {
                    if conflict_with(&child, &protected).is_some() {
                        d_skipped += 1;
                        done += 1;
                        continue;
                    }
                    let size = dir_size(&child);
                    let res = if child.is_dir() { std::fs::remove_dir_all(&child) } else { std::fs::remove_file(&child) };
                    match res {
                        Ok(()) => d_freed += size,
                        Err(_) => d_skipped += 1,
                    }
                    done += 1;
                    if last.elapsed() >= Duration::from_millis(80) {
                        emit_progress(&app, def.label, done, total, freed + d_freed);
                        last = Instant::now();
                    }
                }
            }
            freed += d_freed;
            skipped += d_skipped;
            details.push(CleanDetail { id: def.id.into(), label: def.label.into(), freed: d_freed, skipped: d_skipped });
            emit_progress(&app, def.label, done, total, freed);
        }
        emit_progress(&app, "Done", total, total, freed);
        if freed > 0 {
            record_history("cache", title.unwrap_or_else(|| "Cache cleanup".into()), freed, Vec::new());
        }
        CleanReport { freed, skipped, details }
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(r)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
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
            clean_cache,
            protected_list,
            protect_add,
            protect_remove,
            list_history,
            clear_history,
            undo_entry,
            data_folder,
            open_data_folder,
            settings_load,
            settings_save,
            send_notification,
            remove_broken_entry
        ])
        .run(tauri::generate_context!())
        .expect("error while running Dustoff");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_entries_are_detected_without_false_alarms() {
        // an uninstaller that no longer exists is broken
        assert!(is_broken(r#""C:\definitely\missing\folder\Update.exe" --uninstall"#));
        assert!(is_broken(r"C:\definitely\missing\unins000.exe /SILENT"));
        // MSI uninstalls, existing programs and relative commands are not flagged
        assert!(!is_broken("MsiExec.exe /X{12345678-1234-1234-1234-123456789012}"));
        assert!(!is_broken(r#""C:\Windows\System32\cmd.exe" /c echo hi"#));
        assert!(!is_broken("rundll32.exe shell32.dll,Control_RunDLL"));
        assert!(!is_broken(""));
    }

    #[test]
    fn dates_convert_correctly() {
        assert_eq!(unix_to_date(0), "1970-01-01");
        assert_eq!(unix_to_date(1_700_000_000), "2023-11-14");
        assert_eq!(unix_to_date(1_709_164_800), "2024-02-29");
    }

    #[test]
    fn rot13_decodes_userassist_names() {
        assert_eq!(rot13("Pbqr"), "Code");
        assert_eq!(rot13(&rot13("C:\\Program Files\\App\\app.exe")), "C:\\Program Files\\App\\app.exe");
    }

    #[test]
    fn protected_folders_block_children_and_parents_but_not_lookalikes() {
        let base = std::env::temp_dir().join(format!("dustoff_test_{}", now_millis()));
        let protected = base.join("keep");
        let inside = protected.join("sub");
        let lookalike = base.join("keep_other");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(&lookalike).unwrap();
        let list = vec![protected.to_string_lossy().to_string()];

        assert!(conflict_with(&inside, &list).is_some(), "child of a protected folder must be blocked");
        assert!(conflict_with(&protected, &list).is_some(), "the protected folder itself must be blocked");
        assert!(conflict_with(&base, &list).is_some(), "a parent that contains a protected folder must be blocked");
        assert!(conflict_with(&lookalike, &list).is_none(), "a folder that only shares a name prefix must be allowed");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn real_machine_apps_have_dates() {
        let ua = userassist_entries();
        let apps = read_registry_apps();
        let with_install = apps.iter().filter(|a| !a.info.installed.is_empty()).count();
        let with_used = apps.iter().filter(|a| !last_used_for(a, &ua).is_empty()).count();
        println!("apps: {}, with install date: {}, with last-used: {}, userassist entries: {}", apps.len(), with_install, with_used, ua.len());
        let broken: Vec<String> = apps.iter().filter(|a| a.info.broken).map(|a| format!("{} ({})", a.info.name, a.info.version)).collect();
        println!("broken entries on this PC: {:?}", broken);
        for a in apps.iter().take(8) {
            println!("  {} | installed {} ({}) | last used {}", a.info.name, a.info.installed, if a.info.installed_approx { "approx" } else { "exact" }, last_used_for(a, &ua));
        }
        assert!(!apps.is_empty());
    }

    #[test]
    fn norm_path_ignores_case_slashes_and_verbatim_prefix() {
        assert_eq!(norm_path(Path::new(r"\\?\C:\Users\Test\")), norm_path(Path::new("c:/users/test")));
    }
}

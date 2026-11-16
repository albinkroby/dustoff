# Dustoff

Dustoff is a Windows app that shows what is using your disk space and helps you get it back: browse folders, uninstall apps and clean up what they leave behind, find large and duplicate files, and clear caches. Nothing is deleted without a preview and your confirmation.

Built with [Tauri 2](https://tauri.app) (Rust backend, plain HTML/CSS/JavaScript frontend). It runs entirely on your PC and makes no network connections.

## Features

- **Dashboard** with free space for every drive, one-click **Smart clean** (safe items only, with a preview), and a warning banner when a drive is almost full. While the app is open it rechecks your drives every few minutes and can send a Windows notification when a drive gets fuller than your chosen level.
- **Storage** browser with list and treemap views, an editable path bar, back/forward/up navigation, live parallel scanning, and a right-click menu (open, show in Explorer, copy path, protect, move to the Recycle Bin).
- **Apps**: installed programs with real icons, sizes, install dates and last-used dates. "Uninstall + clean" runs the app's own uninstaller, then offers leftover folders for review. Apps whose uninstaller is already gone are flagged as broken entries and can be removed from Windows' list.
- **Files**: large file finder and exact duplicate finder (matched by content hash). Files go to the Recycle Bin only, and every copy in a group cannot be removed at once.
- **Cleanup**: temp files, thumbnail and shader caches, error reports, npm and pip caches, plus optional browser, Discord and VS Code caches and the Recycle Bin, with a progress bar.
- **Safety**: protected folders Dustoff will never touch, and an activity log of everything it removed, with one-click restore for items in the Recycle Bin.
- **Settings**: theme, start page, default view, cleaning preferences and the nearly-full warning level. Settings, protected folders and the log are stored in `%APPDATA%\Dustoff`.
- Light and dark themes.

## Safety

- Every deletion shows a preview and asks for confirmation.
- Deleting app data is limited to an allow-list of locations (AppData, ProgramData, Program Files, Downloads) and blocks Windows folders and shared publisher folders.
- Folders you protect are never deleted, moved or cleaned, and neither is any folder that contains them.
- Recycle Bin removals are recorded and can be restored from the activity log. Permanent removals (such as caches) are clearly marked as not restorable.
- The frontend never passes arbitrary commands to the backend. Uninstallers are looked up by name from the Windows registry.

## Build from source

Requirements (Windows 10/11, x64):

- [Node.js](https://nodejs.org) 20 or newer
- [Rust](https://rustup.rs) (stable, MSVC toolchain)
- Visual Studio Build Tools with the "Desktop development with C++" workload
- WebView2 runtime (included with Windows 11 and current Windows 10)

```powershell
npm install
npm run dev      # run in development mode
npm run build    # produce the installers in src-tauri/target/release/bundle
```

## Releasing

Releases are built by GitHub Actions (`.github/workflows/release.yml`) and only from the `main` branch:

1. Update `version` in `src-tauri/tauri.conf.json` (and `src-tauri/Cargo.toml`), commit, and merge to `main`.
2. Tag the commit on `main` and push the tag:
   ```powershell
   git checkout main
   git pull
   git tag v0.1.0
   git push origin v0.1.0
   ```
3. The workflow checks that the tag is on `main` and matches the app version, builds the Windows installers, and creates a **draft** release. Review it on the Releases page and press Publish.

If the tag is not on `main` or does not match the version, the workflow stops and no release is created.

## Project layout

```
src/          Frontend (index.html, style.css, main.js)
src-tauri/    Rust backend (src/lib.rs), Tauri config, icons
app-icon.png  Source image used to generate the icon set (npx tauri icon app-icon.png)
```

## Notes

- Windows only.
- Release installers are not code-signed, so Windows SmartScreen may show a warning on first run.
- Some operations (for example parts of `C:\Windows\Temp`, or uninstalling some programs) need administrator rights. Run Dustoff as administrator for those.

## License

[MIT](LICENSE)

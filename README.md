# Dustoff

Dustoff is a Windows app that shows what is using your disk space and helps you get it back: browse folders, uninstall apps and clean up what they leave behind, find large and duplicate files, and clear caches. Nothing is deleted without a preview and your confirmation.

Built with [Tauri 2](https://tauri.app) (Rust backend, plain HTML/CSS/JavaScript frontend). It runs entirely on your PC and makes no network connections.

## Features

- **Dashboard** with free space for every drive, and a warning when a drive is almost full.
- **Storage** browser with list and treemap views, an editable path bar, back/forward/up navigation, and live parallel scanning.
- **Apps**: installed programs with real icons and sizes. "Uninstall + clean" runs the app's own uninstaller, then offers leftover folders for review. Removed folders go to the Recycle Bin unless you choose permanent deletion.
- **Files**: large file finder and exact duplicate finder (matched by content hash). Files go to the Recycle Bin only, and every copy in a group cannot be removed at once.
- **Cleanup**: temp files, thumbnail and shader caches, error reports, npm and pip caches, plus optional browser, Discord and VS Code caches and the Recycle Bin.
- Light and dark themes.

## Safety

- Every deletion shows a preview and asks for confirmation.
- Deleting app data is limited to an allow-list of locations (AppData, ProgramData, Program Files, Downloads) and blocks Windows folders and shared publisher folders.
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

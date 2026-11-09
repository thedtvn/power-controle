# Power Control

Native Windows **GPUI** tray application for Windows power plans and Dell
thermal/battery charging settings. The former Tauri/React interface has been
replaced; Node.js, a webview, and frontend build tools are not required.

## Build

Requires Windows, stable Rust with the MSVC toolchain, Visual Studio C++ build
tools and the Windows SDK. GPUI uses native GPU rendering, so a working graphics
driver is required.

```powershell
cargo build --release --locked
```

The executable is `target/release/power-controle.exe`. The historical
`src-tauri` directory now contains the native Rust application; it is a member
of the root Cargo workspace, not a Tauri project.

Release builds need administrator rights for Dell controls. GPUI embeds the
executable's only Windows manifest (DPI awareness), so instead of a
`requireAdministrator` manifest the release app relaunches itself through UAC
when started unelevated. If you decline UAC it keeps running: power plans still
work, and Dell operations report an error. The Start with Windows task already
runs elevated, so login launches show no prompt. Development builds never
request elevation.

## Installers and GitHub releases

Every push runs `.github/workflows/release.yml` on `windows-latest`. It:

1. runs Rust tests and the mocked startup tests;
2. builds `power-controle.exe` in release mode;
3. packages a per-machine **MSI** (WiX 5, `installer/power-control.wxs`) and a
   **setup EXE** (NSIS, `installer/power-control.nsi`), both installing to
   `Program Files\Power Control`;
4. uploads the MSI, setup EXE, portable EXE, and `SHA256SUMS.txt` as workflow
   artifacts and publishes them as a GitHub release tagged
   `v<version>-build.<run number>`.

The version comes from `src-tauri/Cargo.toml` and must be numeric `x.y.z` for
the MSI. Use **one** installer type per machine: the MSI and setup EXE are
separate installers. Portable copies cannot enable elevated startup.

To build installers locally after `cargo build --release --locked`:

```powershell
makensis /DVERSION=0.1.0 installer/power-control.nsi
dotnet tool install --global wix --version 5.0.2
wix build installer/power-control.wxs -arch x64 -d Version=0.1.0 -d ExePath=$PWD\target\release\power-controle.exe -d IconPath=$PWD\src-tauri\icons\icon.ico -o target\release\Power-Control-0.1.0-x64.msi
```

Before updating or uninstalling, quit the running copy using its tray menu;
disable Start with Windows before uninstalling. Uninstalling keeps user
settings and saved Credential Manager entries.

## Existing settings and credentials

The migration retains `%APPDATA%\com.duong.power-controle\config.json`, including
legacy JSON configuration migration. BIOS passwords remain in Windows
Credential Manager under the existing `power-controle/dell-cctk-bios-password`
identity. They are not stored in configuration JSON or logged.

## Start with Windows

Install the release executable **for all users under Program Files** before
enabling **Start with Windows**. Accept its administrator prompt when opening
it manually. Elevated startup is deliberately refused from Desktop, Downloads,
AppData, or other user-writable locations: otherwise another process could
replace the executable and gain administrator access at your next login.

Startup uses a Windows Task Scheduler task named `PowerControl-<your Windows SID>`:

- Runs only when that user signs in, in their interactive desktop session.
- Runs with highest privileges, preserving access to Dell controls without a
  fresh UAC prompt at each login (subject to Windows account/security policies).
- Waits 15 seconds after sign-in to let the desktop and notification area start.
- Runs on battery, has no execution time limit, and ignores duplicate launches.

The app migrates enabled legacy `power-controle` Run entries on launch. It
creates and verifies the replacement task **before** removing those entries.
Disabled entries are not automatically enabled. Disabling startup removes the
app's task and owned legacy Run entries; other applications are untouched.
A moved installation's enabled task is repaired before its fallback is removed.

Hardware discovery and system changes run on a serialized background worker,
not the UI thread. A scheduled duplicate launch stays quiet; manually opening
the app again brings up Settings. Closing Settings should leave the tray running;
use its Quit menu item to exit completely.

### Manual installed-release check

1. Exit the old app from its tray menu, then install/open the updated release
   from Program Files. Keep only one running copy.
2. Enable **Start with Windows**. Toggle off/on to refresh registration if needed.
3. In Task Scheduler Library, verify `PowerControl-<your SID>` uses **At log on**,
   delayed 15 seconds, **Run only when user is logged on**, and highest privileges.
   Do not change it to SYSTEM or “whether user is logged on or not”: those modes
   cannot show a tray icon in your desktop.
4. Sign out and back in (or restart), wait at least 15 seconds, and check the
   notification area's hidden-icons overflow too. Windows controls whether the
   icon is pinned beside the clock.
5. Check tray plan selection, Settings reopen, native controls, saved settings,
   and any Dell features supported by your hardware. An unsupported Dell control
   must show a useful unavailable/error state rather than pretend success.
6. Disable startup and confirm no launch at the next sign-in.

## Tests

```powershell
cargo check --locked
cargo test --locked --lib
cargo run -- --smoke-test
powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File src-tauri/tests/startup.Tests.ps1
```

The smoke test opens Settings, closes it, verifies the tray process survives,
reopens Settings, and exits after five seconds. It does not start the system
worker or apply hardware/startup settings. Run it with no other app copy running.

Startup tests use mocks for mutations. A Rust integration check queries real
Windows startup state read-only; it does not register tasks, delete registry
entries, change power plans, or write BIOS settings. Automated checks cannot
establish tray visibility at a real Windows sign-in; use the manual check above.

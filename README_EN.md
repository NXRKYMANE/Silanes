# 🎉 Silanes — Windows Service Generator Framework

<p align="center">
  <img src="https://img.shields.io/github/followers/NXRKYMANE?style=social" />
  <img src="https://img.shields.io/github/forks/NXRKYMANE/Silanes" />
  <img src="https://img.shields.io/github/stars/NXRKYMANE/Silanes" />
  <img src="https://img.shields.io/badge/-Rust-000000?style=flat&logo=rust&logoColor=white" />
  <img src="https://img.shields.io/badge/Douyin-Ozones-000000?style=flat&logo=tiktok&logoColor=white" />
  <img src="https://img.shields.io/badge/QQ-946777609-12B7F5?style=flat&logo=tencentqq&logoColor=white" />
</p>

Register any executable as a Windows system service via YAML config. [中文文档](README.md)

> Based on [WinSW 2](https://github.com/winsw/winsw).
> Silanes retains the core service-wrapper concept, implemented in **Rust** with ongoing maintenance and feature extensions.

## ⚡ Rust Implementation

Silanes is implemented in Rust (edition 2024) and compiled into a self-contained, single-file `silanes64.exe`:

| Item | Detail |
|---|---|
| Implementation | Rust (edition 2024, self-contained, single file) |
| Artifact | `rust\publish\silanes64.exe` |
| Size | ~3.5 MB |
| Installer | `silanes-win-x64-setup-v<VERSION>.exe` |
| Build tools | Rust stable + MSVC |

## 🚀 Quick Start

```powershell
# Install (requires administrator)
sil --install <svc.yaml>

# Manage (the sil alias is available after a framework install; the -m prefix is optional)
sil --start     <my-service>
sil --stop      <my-service>
sil --restart   <my-service>
sil --status    <my-service>
sil --uninstall <my-service>
sil --delete    <my-service>
sil --list

# Show help
sil help
```

## 📋 Commands

| Command | Usage |
|---|---|
| `--install <yaml>` | Install / update a service |
| `--uninstall <name>` | Stop and uninstall |
| `--start <name>` | Start a service |
| `--stop <name>` | Stop a service |
| `--restart <name>` | Restart a service |
| `--status <name>` | Query service status |
| `--delete <name>` | Force delete (stop + uninstall) |
| `--list` | List all platform-deployed services (excludes inplace standalone services) |
| `help` / `-h` / `--help` | Print help text |

> Management commands are equivalent to the legacy `-m --xxx` form — the `-m` prefix is optional. After a framework install (`%ProgramFiles%\Silanes` is on PATH) you can use the `sil` shortcut alias instead of `silanes64.exe`.

> `--install-updater` / `--uninstall-updater` are internal commands invoked with the `-internal` prefix (not `-m`), used by the installer to register / remove the Service Updater.
> The service name `Silanes Service Updater` is reserved; service names are validated: empty names, `.` / `..` (path traversal), path separators and control characters are rejected, length capped at 256.

## ⚙️ Config Reference

### 📌 Required Fields

```yaml
service_name: my-service
service_display_name: My Service
service_description: Description of the service
service_executable_path: C:\app\myapp.exe
```

### 🔧 Optional Fields — Basics

| Field | Type | Default | Description |
|---|---|---|---|
| `service_executable_args` | string | `""` | Command-line arguments for the target executable (passed through verbatim, quotes preserved) |
| `service_start_mode` | string | `"automatic"` | Startup type: `automatic`, `delayed_auto`, `manual`, `disabled` |
| `service_dependencies` | string | none | Semicolon-separated list of services that must start first (e.g. `"EventLog;WinRM"`) |
| `service_account` | string | `LocalSystem` | Windows account to run the service as (e.g. `"NT AUTHORITY\\NetworkService"`) |
| `service_password` | string | `""` | Password for `service_account` (only needed for user accounts) |
| `env` | object | none | Environment variables injected into the target process |

### 🔄 Optional Fields — Lifecycle & Hooks

| Field | Type | Default | Description |
|---|---|---|---|
| `failure_reset_sec` | int | `86400` | Failure counter reset period in seconds |
| `restart_delay_ms` | int | `60000` | Delay before auto-restart after crash in milliseconds |
| `kill_process_tree` | bool | `true` | Whether to force-kill the whole process tree on stop |
| `prestart_command` | string | none | Hook run before launching the target (`cmd /c` semantics, failure is non-fatal; killed after 60s timeout) |
| `poststop_command` | string | none | Hook run after the target stops (injects `WINSGF_CHILD_PID` / `WINSGF_CHILD_EXIT_CODE`) |

### ⬇️ Optional Fields — Pre-Start Download

| Field | Type | Default | Description |
|---|---|---|---|
| `download_url` | string | none | URL to fetch the target executable before launch |
| `download_to` | string | none | Download destination; relative paths resolve against the service directory |
| `download_sha256` | string | none | SHA-256 of the downloaded file (lowercase hex) |
| `download_fail_on_error` | bool | `true` | Whether a failed download fails service startup |

> Security note: with `http://` and no `download_sha256`, `fail_on_error=true` refuses to start (protects against tampering in transit).

### 📝 Optional Fields — Logging

| Field | Type | Default | Description |
|---|---|---|---|
| `log_enabled` | bool | `true` | Whether host logs are written |
| `log_dir` | string | none | Log directory; relative paths resolve against the service directory |
| `log_max_size_mb` | int | `0` | Max log file size (MB) before rollover; `0` means unlimited |
| `log_max_backup_count` | int | `5` | Number of rolled-over backups to keep |
| `log_split_out_err` | bool | `false` | Write child stderr to a separate `yyyy-MM-dd.err.log` |

### 📦 Optional Fields — Standalone Mode (inplace)

| Field | Type | Default | Description |
|---|---|---|---|
| `deploy_inplace` | bool | `false` | Register the current `silanes64.exe` in place instead of deploying to ProgramData; the YAML must be named after the exe and sit next to it (use the actual exe file name). Intended for embedding Silanes inside your own project; excluded from boot-time host upgrades and cleanup — upgrade the framework manually from the official Releases |

### 📄 Full Example

```yaml
service_name: my-service
service_display_name: My Service
service_description: My application service
service_executable_path: C:\app\myapp.exe
service_executable_args: --mode production
service_start_mode: delayed_auto
service_dependencies: EventLog;WinRM
service_account: NT AUTHORITY\NetworkService
env:
  MY_VAR: value
  LOG_LEVEL: info
failure_reset_sec: 86400
restart_delay_ms: 60000
kill_process_tree: true
prestart_command: echo pre-start >> C:\app\hook.log
poststop_command: echo child=%WINSGF_CHILD_PID% >> C:\app\hook.log
download_url: https://example.com/app.exe
download_to: C:\app\myapp.exe
download_sha256: <sha256>
download_fail_on_error: true
log_enabled: true
log_dir: logs
log_max_size_mb: 10
log_max_backup_count: 5
log_split_out_err: true
```

## 🔍 How It Works

1. **Install**: Silanes deploys a copy of itself alongside the YAML config to `C:\ProgramData\Silanes\svcs\<name>\` (directory ACL is tightened to SYSTEM / Administrators only) and registers it via the SCM API. Reinstalling an existing name compares the source (executable path + arguments); a different source is rejected to avoid hijacking an unrelated service.
2. **Runtime**: When SCM starts the service, Silanes reads the YAML config and launches the target as a child process. If `download_url` is set, the target file is ensured to be ready before launch (with SHA-256 verification).
3. **Logging**: Child stdout/stderr and host lifecycle events are written to `logs\yyyy-MM-dd.log` (concurrent writes serialized by a mutex; size rollover and stderr splitting supported).

### ♻️ Service Recovery

- SCM layer: on a crash the service restarts after `restart_delay_ms` (up to 2 times), with the failure counter reset after `failure_reset_sec`;
- Host layer: when the child exits with a **non-zero exit code**, the host restarts it automatically (up to 3 times) and stops the service once the limit is exceeded.

### 🪝 Hooks

- **prestart** (`prestart_command`): runs before the target is launched, `cmd /c` semantics (pipes / redirection supported); failure is non-fatal, and a 60-second timeout force-kills the whole hook tree (so a stuck hook cannot trip the SCM 30-second startup timeout).
- **poststop** (`poststop_command`): runs after the target stops, with `WINSGF_CHILD_PID` / `WINSGF_CHILD_EXIT_CODE` injected; failure is only a warning.

### 🌙 Graceful Shutdown

On stop or system shutdown: GUI processes receive `WM_CLOSE` (sent to every top-level window) → console processes receive `Ctrl+C` (broadcast to the shared console; the host registers an ignore handler to avoid killing itself) → after a 10-second timeout the process is force-killed; `kill_process_tree=true` (default) also terminates the whole tree.

### 🧩 Standalone Mode (inplace)

With `deploy_inplace: true`, `--install` registers the current exe **in place**:

- No copy to ProgramData; the ImagePath points directly at the current exe;
- `service_name` must equal the actual exe file name (e.g. `silanes64`; if you rename the exe, use its actual name), otherwise SCM cannot dispatch the service;
- Designed for embedding Silanes into your own project; excluded from boot-time host upgrades and cleanup. Developers must manually upgrade `silanes64.exe` from the [official Releases](https://github.com/NXRKYMANE/Silanes/releases).

### 📡 Service Updater

The installer automatically registers a **Service Updater** (`Silanes Service Updater`) that upgrades all registered service hosts after system boot and cleans up residue:

1. **Registration (install time)** — The Inno Setup installer calls `silanes64.exe -internal --install-updater`, registering itself with the `-internal --updater` parameter as a Windows service with "Automatic (Delayed Start)" so host services start before the upgrade scan.
2. **Boot-time execution** — About 2 minutes after system startup, SCM launches the Service Updater. It scans `C:\ProgramData\Silanes\svcs\` for all hosts: a host version **lower** than the installed one → stop, replace the binary, restart; equal or higher → skip.
3. **Stale-service cleanup** — Removes services with a missing YAML, nonexistent target, or unparsable config (plus their host directories), and orphaned directories (SCM record gone but the `svcs` folder remains).
4. **Log cleanup** — Deletes logs older than 30 days in each service's log directory and the updater's own (`%ProgramData%\Silanes\updater\`), including `.err.log` split logs and `.N` rollover backups.
5. **Auto-stop** — The service stops itself after one full scan; it does not stay resident.
6. **Removal (uninstall time)** — The Inno Setup uninstaller calls `silanes64.exe -internal --uninstall-updater` to stop and remove the service.

> A **system reboot** is required after installation for the Service Updater to run on the next boot.

## 🏗️ Build

The one-click build script produces 2 artifacts (executable + installer):

```powershell
.\BUILD.ps1
```

**Pipeline**: build Rust → Rust unit tests → compile the installer with ISCC (Inno Setup 7).

The script reads the version from `rust\Cargo.toml` and automatically syncs it (plus the copyright year) into `installer.iss`. A failing test aborts the pipeline; use `.\BUILD.ps1 -SkipTests` to skip testing.

### 🛠️ Build Individually

```powershell
Set-Location rust
cargo build --release                    # → rust\target\release\silanes64.exe
Copy-Item target\release\silanes64.exe publish\silanes64.exe
ISCC installer.iss                    # → rust\publish\silanes-win-x64-setup-v<VERSION>.exe
```

## 💿 Installer Deployment

Pre-built installers are available on the [Releases](https://github.com/NXRKYMANE/Silanes/releases) page.

### 📦 Installer

| Installer | Notes |
|---|---|
| `silanes-win-x64-setup-v<VERSION>.exe` | Standard installer |

The installer places `silanes64.exe` in `%ProgramFiles%\Silanes\` and registers the Control Panel uninstall entry and the boot-time Service Updater.

### ✨ Installer Features

- Installs `silanes64.exe` to `%ProgramFiles%\Silanes\` and adds it to the system PATH
- Includes README documentation (English and Chinese) in HTML format
- Automatically registers the boot-time Service Updater (`--install-updater`)
- Registers an uninstall entry in Windows Control Panel
- Auto-detects old versions: silently upgrades on newer, prompts to reinstall on identical, warns on downgrade
- Finish page offers an optional checkbox to open the README; prompts to reboot after installation

### ⚠️ Inno Setup Integration Tips

When embedding Silanes in your own Inno Setup installer, watch out for these common pitfalls:

1. **YAML Backslashes** — The install directory (e.g. `C:\Program Files\ASMMS`) contains backslashes. In YAML, unquoted values handle backslashes natively — simply avoid wrapping paths in quotes.
2. **Silanes Not in PATH** — The installer process may not have `silanes64.exe` in PATH even after installation. Read the full path from registry: `HKLM\Software\Microsoft\Windows\CurrentVersion\App Paths\silanes64.exe`.
3. **Elevated Child Fails to Start** — Inno's `Exec` returns `ERROR_ACCESS_DENIED` when directly starting a requireAdministrator child; route through `cmd.exe`: `Exec('{sys}\cmd.exe', '/c ""<exe>" <args>"', '', SW_HIDE, ewWaitUntilTerminated, ...)`.
4. **Silent Install Language Dialog** — With multiple languages, `/VERYSILENT` silent installs must pass `/LANG=` explicitly (`/LANG` takes precedence and suppresses the dialog); otherwise the language dialog still pops up and hangs.

## 📁 Project Structure

```
Silanes/
├── rust/                      # Rust implementation
│   ├── main.rs                # Entry: argument parsing and routing
│   ├── service_core.rs        # Core: CLI, SCM API, Service Updater
│   ├── service_host.rs        # Service host: launches target process + graceful stop
│   ├── service_config.rs      # YAML config model (serde)
│   ├── service_tests.rs       # Unit tests (38 tests, incl. process-tree integration test)
│   ├── build.rs               # EXE version info / icon / language metadata (winresource)
│   ├── Cargo.toml             # Project config (release speed optimizations)
│   ├── Cargo.lock             # Dependency lock file
│   └── installer.iss          # Inno Setup install script
├── docs/                      # Chinese & English HTML docs
│   ├── README_CN.html         # Chinese HTML documentation (shipped with installers)
│   └── README_EN.html         # English HTML documentation (shipped with installers)
├── misc/                      # Misc resources
│   ├── sil.cmd                # sil shortcut alias (copied by the installer)
│   └── images/                # Icons and images
│       ├── Proj.ico           # Program icon (installer + distribution)
│       ├── Proj.png           # Project icon
│       ├── Background.bmp     # Installer wizard background
│       ├── Rust.bmp           # Installer wizard small image
│       └── Rust.png           # Project image
├── BUILD.ps1                  # One-click build script (Rust build + tests + installer)
├── .github/                   # GitHub community templates (issues / PR)
├── AGENTS.md                  # AI collaboration rules
├── CODE_OF_CONDUCT.md         # Code of conduct
├── CONTRIBUTING.md            # Contributing guidelines
├── SECURITY.md                # Security policy
├── LICENSE                    # License
├── README.md                  # Chinese documentation
└── README_EN.md               # English documentation
```

## 🧪 Testing

Rust automated tests cover input validation, startup-mode parsing, log cleanup, process-tree collection, ACL permission checks, downloading, and other core logic:

```powershell
# Rust (38 tests, incl. a real process-tree integration test)
Set-Location rust
cargo test
```

- Tests are consolidated in `rust\service_tests.rs`; the test build never ships in the release binary;
- Security boundaries such as path traversal, control-character injection, and SDDL permission checks are covered.

## ✅ Requirements

- Windows 10+ x64
- Administrator privileges
- Build tools (build only):
  - Rust stable (edition 2024) + MSVC linker (Visual Studio C++ Build Tools) — to build the Rust binary
  - Inno Setup 7 — to build the installer (default path `C:\Program Files\Inno Setup 7\ISCC.exe`)

## 💖 Sponsor

If this project helps you, consider [sponsoring](https://ifdian.net/a/NXRKYMANE).

## 📄 License

Copyright © 2026 NXRKYMANE SOFTWARE

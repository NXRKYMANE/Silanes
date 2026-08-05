# 🎉 Silanes — Windows Service Generator Framework

<p align="center">
  <img src="https://img.shields.io/github/followers/NXRKYMANE?style=social" />
  <img src="https://img.shields.io/github/forks/NXRKYMANE/Silanes" />
  <img src="https://img.shields.io/github/stars/NXRKYMANE/Silanes" />
  <img src="https://img.shields.io/badge/-Rust-000000?style=flat&logo=rust&logoColor=white" />
  <img src="https://img.shields.io/badge/Douyin-Ozones-000000?style=flat&logo=tiktok&logoColor=white" />
  <img src="https://img.shields.io/badge/QQ-946777609-12B7F5?style=flat&logo=tencentqq&logoColor=white" />
  <img src="https://komarev.com/ghpvc/?username=NXRKYMANE&repo=Silanes&label=Views&color=00BFFF&style=flat" />
</p>

将任意可执行程序注册为 Windows 系统服务。 [SEE ENGLISH DOCS](README_EN.md)

> 基于 [WinSW 2](https://github.com/winsw/winsw)。
> Silanes 保留核心服务包装概念，以 **Rust** 实现，持续维护扩展。

> 我只是一个普通高中生，项目初期功能和漏洞波动较大，更新频率也非常高，望请各位开发者大佬谅解。

## ⚡ Rust 实现

Silanes 由 Rust（edition 2024）实现，编译为自包含单文件 `silanes64.exe`：

| 项 | 说明 |
|---|---|
| 实现 | Rust（edition 2024，自包含、单文件） |
| 产物 | `rust\publish\silanes64.exe` |
| 体积 | 约 3.5 MB |
| 安装包 | `silanes-win-x64-setup-v<版本>.exe` |
| 构建工具 | Rust stable + MSVC |

## 🚀 快速开始

```powershell
# 安装服务（需管理员权限）
sil --install <svc.yaml>

# 管理服务（框架安装后可用 sil 快捷别名，-m 前缀可省略）
sil --start     <my-service>
sil --stop      <my-service>
sil --restart   <my-service>
sil --status    <my-service>
sil --uninstall <my-service>
sil --delete    <my-service>
sil --list

# 查看帮助
sil help
```

## 📋 命令列表

| 命令 | 说明 |
|---|---|
| `--install <yaml>` | 安装 / 更新服务 |
| `--uninstall <名称>` | 停止并卸载服务 |
| `--start <名称>` | 启动服务 |
| `--stop <名称>` | 停止服务 |
| `--restart <名称>` | 重启服务 |
| `--status <名称>` | 查询服务状态 |
| `--delete <名称>` | 强制删除（停止 + 卸载） |
| `--list` | 列出平台部署的所有服务（不含 inplace 独立服务） |
| `help` / `-h` / `--help` | 显示帮助信息 |

> 管理命令均等价于 `-m --xxx` 旧写法，`-m` 前缀可省略；框架安装后（`%ProgramFiles%\Silanes` 已加入 PATH）可直接用 `sil` 快捷别名代替 `silanes64.exe`。

> `--install-updater` / `--uninstall-updater` 为内部命令，须以 `-internal` 前缀调用，用于安装包注册 / 移除服务更新程序。
> 服务名 `Silanes Service Updater` 为保留名；服务名需合法：拒绝空名、`.` / `..`（防路径穿越）、路径分隔符与控制字符，长度 ≤ 256。

## ⚙️ 配置参考

### 📌 必填字段

```yaml
service_name: my-service
service_display_name: My Service
service_description: 服务描述
service_executable_path: C:\app\myapp.exe
```

### 🔧 可选字段 — 基础

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `service_executable_args` | string | `""` | 目标程序命令行参数（原样拼接，保留引号语义） |
| `service_start_mode` | string | `"automatic"` | 启动类型：`automatic`、`delayed_auto`、`manual`、`disabled` |
| `service_dependencies` | string | 无 | 依赖服务名列表，分号分隔（如 `"EventLog;WinRM"`） |
| `service_account` | string | `LocalSystem` | 服务运行账户（如 `"NT AUTHORITY\\NetworkService"`） |
| `service_password` | string | `""` | 服务账户密码（仅自定义账户需要） |
| `env` | object | 无 | 注入目标进程的环境变量 |

### 🔄 可选字段 — 生命周期与钩子

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `failure_reset_sec` | int | `86400` | 失败计数重置周期（秒） |
| `restart_delay_ms` | int | `60000` | 崩溃后自动重启延迟（毫秒） |
| `kill_process_tree` | bool | `true` | 停止时是否强制终止整棵进程树 |
| `prestart_command` | string | 无 | 启动前钩子（`cmd /c` 语义，失败不阻断；超时 60s 强杀） |
| `poststop_command` | string | 无 | 停止后钩子（注入 `WINSGF_CHILD_PID` / `WINSGF_CHILD_EXIT_CODE`） |

### ⬇️ 可选字段 — 启动前下载

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `download_url` | string | 无 | 启动前下载目标可执行文件的 URL |
| `download_to` | string | 无 | 下载目标路径；相对路径基于服务部署目录 |
| `download_sha256` | string | 无 | 下载文件 SHA-256（小写十六进制） |
| `download_fail_on_error` | bool | `true` | 下载失败是否导致服务启动失败 |

> 安全提示：`http://` 且未提供 `download_sha256` 时，`fail_on_error=true` 直接拒绝启动（防明文传输被篡改）。

### 📝 可选字段 — 日志

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `log_enabled` | bool | `true` | 是否写入服务日志 |
| `log_dir` | string | 无 | 日志目录；相对路径基于服务部署目录 |
| `log_max_size_mb` | int | `0` | 单日志大小上限（MB），超过滚动备份；`0` 不限 |
| `log_max_backup_count` | int | `5` | 滚动保留的备份份数 |
| `log_split_out_err` | bool | `false` | 子进程 stderr 单独写入 `yyyy-MM-dd.err.log` |

### 📦 可选字段 — 独立模式（inplace）

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `deploy_inplace` | bool | `false` | 原地注册：不复制宿主到 ProgramData，直接用当前 `silanes64.exe` 注册；YAML 必须与 exe 同名同目录（以实际 exe 文件名为准）。适合嵌入自有项目独立使用；不参与开机宿主升级与清理，框架升级需自行到官网 Releases 下载新版 `silanes64.exe` |

### 📄 完整示例

```yaml
service_name: my-service
service_display_name: My Service
service_description: 我的应用程序服务
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

## 🔍 工作原理

1. **安装**：Silanes 将自身副本和 YAML 部署到 `C:\ProgramData\Silanes\svcs\<名称>\`（目录 ACL 收紧，仅 SYSTEM / Administrators 可写），经 SCM 注册为服务。重复安装同名服务时比对来源（可执行路径 + 参数），来源不同则拒绝覆盖。
2. **运行时**：SCM 启动服务时读取 YAML 并拉起目标进程；若配置 `download_url`，启动前先确保目标文件就绪（含 SHA-256 校验）。
3. **日志**：子进程 stdout/stderr 与宿主生命周期事件写入 `logs\yyyy-MM-dd.log`（互斥串行化；支持大小滚动与 stderr 分流）。

### ♻️ 服务恢复

- SCM 层：目标进程崩溃后按 `restart_delay_ms` 延迟自动重启（最多 2 次），失败计数在 `failure_reset_sec` 周期后重置；
- 宿主层：子进程**非零退出码**异常退出时自动重启（最多 3 次），超限则停止服务。

### 🪝 钩子（Hooks）

- **prestart**（`prestart_command`）：拉起目标前执行，`cmd /c` 语义支持管道 / 重定向；失败不阻断，超时 60 秒强杀（防止钩子卡死触发 SCM 30 秒启动超时）。
- **poststop**（`poststop_command`）：目标停止后执行，注入 `WINSGF_CHILD_PID` / `WINSGF_CHILD_EXIT_CODE`；失败仅告警。

### 🌙 优雅关闭

停止服务或关机时：GUI 程序接收 `WM_CLOSE`（枚举全部顶层窗口）→ 控制台程序接收 `Ctrl+C`（广播到共享控制台，宿主注册忽略处理器防误杀）→ 10 秒超时后强杀；`kill_process_tree=true`（默认）连整棵进程树一并终止。

### 🧩 独立模式（inplace）

`deploy_inplace: true` 时 `--install` 把当前 exe **原地注册**为服务：

- 不复制宿主到 ProgramData，ImagePath 直接指向当前 exe；
- `service_name` 必须等于实际 exe 文件名（如 `silanes64`，exe 改名则以其实际文件名为准），否则 SCM 无法分派；
- 适合嵌入自有项目独立使用；不参与开机宿主升级与清理，需开发者自行到[官网 Releases](https://github.com/NXRKYMANE/Silanes/releases) 下载新版 `silanes64.exe` 手动升级。

### 📡 服务更新程序

安装包会自动注册 **服务更新程序**（`Silanes Service Updater`），开机后自动升级所有已注册的服务宿主并清理残留：

1. **注册（安装时）** — Inno Setup 安装程序调用 `silanes64.exe -internal --install-updater`，以 `-internal --updater` 参数注册为「自动（延迟启动）」服务，确保宿主服务先于升级扫描启动。
2. **开机执行** — 系统启动约 2 分钟后扫描 `C:\ProgramData\Silanes\svcs\` 下所有宿主：宿主版本**低于**当前安装版本 → 停止、替换二进制、重启；持平或更高 → 跳过。
3. **清理失效服务** — 移除 yaml 缺失 / 目标不存在 / 配置解析失败的服务及其宿主目录，并清理 SCM 无记录但 `svcs` 仍存在的孤儿目录。
4. **日志清理** — 删除各服务日志及更新程序自身日志（`%ProgramData%\Silanes\updater\`）中超过 30 天的文件（含 `.err.log` 分流与 `.N` 滚动备份）。
5. **自动停止** — 一轮扫描后自动停止，不常驻后台。
6. **移除（卸载时）** — Inno Setup 卸载程序调用 `silanes64.exe -internal --uninstall-updater` 停止并移除该服务。

> 安装完成后需**重启系统**，更新程序才会在下次开机首次运行。

## 🏗️ 构建

一键构建产出全部 2 个产物（exe + 安装包）：

```powershell
.\BUILD.ps1
```

**流水线**：构建 Rust → Rust 单元测试 → ISCC 编译安装包（Inno Setup 7）。

脚本从 `rust\Cargo.toml` 读取版本号，自动同步到 `installer.iss`（含版权年份）。测试失败会终止流水线；跳过测试用 `.\BUILD.ps1 -SkipTests`。

### 🛠️ 单独构建

```powershell
Set-Location rust
cargo build --release                    # → rust\target\release\silanes64.exe
Copy-Item target\release\silanes64.exe publish\silanes64.exe
ISCC installer.iss                    # → rust\publish\silanes-win-x64-setup-v<版本>.exe
```

## 💿 安装包部署

预构建的安装包可在 [Releases](https://github.com/NXRKYMANE/Silanes/releases) 页面获取。

### 📦 安装包

| 安装包 | 说明 |
|---|---|
| `silanes-win-x64-setup-v<版本>.exe` | 标准安装包 |

安装包将 `silanes64.exe` 安装到 `%ProgramFiles%\Silanes\`，注册控制面板卸载条目与开机服务更新程序。

### ✨ 安装器特性

- 将 `silanes64.exe` 安装到 `%ProgramFiles%\Silanes\` 并加入系统 PATH
- 附带 HTML 格式中英文 README
- 自动注册开机服务更新程序（`--install-updater`）
- 注册控制面板卸载条目
- 自动检测旧版本：高版本静默升级、同版本询问重装、低版本警告降级
- 完成页可选在浏览器中打开 README；安装完成后提示重启系统

### ⚠️ Inno Setup 集成注意事项

在自己的 Inno Setup 安装包中嵌入 Silanes 时，注意以下常见问题：

1. **YAML 路径反斜杠** — 安装目录（如 `C:\Program Files\ASMMS`）含反斜杠，YAML 中不加引号直接书写即可，避免引号包裹导致反斜杠被误解析。
2. **Silanes 未加入 PATH** — 安装后安装进程可能仍无法通过 PATH 找到 `silanes64.exe`，应从注册表读取完整路径：`HKLM\Software\Microsoft\Windows\CurrentVersion\App Paths\silanes64.exe`。
3. **提权子进程启动失败** — Inno 的 `Exec` 直接启动 requireAdministrator 子进程会返回 `ERROR_ACCESS_DENIED`，需经 `cmd.exe` 中转：`Exec('{sys}\cmd.exe', '/c ""<exe>" <args>"', '', SW_HIDE, ewWaitUntilTerminated, ...)`。
4. **静默安装语言弹窗** — 多语言安装包 `/VERYSILENT` 静默安装必须显式传 `/LANG=`（`/LANG` 优先级最高，传了不弹框）；否则语言选择框仍会弹出导致卡住。

## 📁 项目结构

```
Silanes/
├── rust/                      # Rust 实现
│   ├── main.rs                # 入口：参数解析与路由
│   ├── service_core.rs        # 核心：CLI、SCM API、服务更新程序
│   ├── service_host.rs        # 服务宿主：拉起目标进程 + 优雅停止
│   ├── service_config.rs      # YAML 配置模型（serde）
│   ├── service_tests.rs       # 单元测试（38 个，含进程树集成测试）
│   ├── build.rs               # EXE 版本信息 / 图标 / 语言元数据（winresource）
│   ├── Cargo.toml             # 项目配置（release 速度优化）
│   ├── Cargo.lock             # 依赖锁定文件
│   └── installer.iss          # Inno Setup 安装脚本
├── docs/                      # 中英文 HTML 文档
│   ├── README_CN.html         # 中文 HTML 文档（随安装包分发）
│   └── README_EN.html         # 英文 HTML 文档（随安装包分发）
├── misc/                      # 杂项资源
│   ├── sil.cmd                # sil 快捷别名（安装器复制到安装目录）
│   └── images/                # 图标与图片
│       ├── Proj.ico           # 程序图标（安装器 + 分发）
│       ├── Proj.png           # 项目图标
│       ├── Background.bmp     # 安装向导背景图
│       ├── Rust.bmp           # 安装向导小图
│       └── Rust.png           # 项目图片
├── BUILD.ps1                  # 一键构建脚本（Rust 构建与测试 + 安装包）
├── .github/                   # GitHub 社区模板（Issue / PR）
├── AGENTS.md                  # AI 协作规则
├── CODE_OF_CONDUCT.md         # 行为准则
├── CONTRIBUTING.md            # 贡献指南
├── SECURITY.md                # 安全政策
├── LICENSE                    # 许可证
├── README.md                  # 中文文档
└── README_EN.md               # 英文文档
```

## 🧪 测试

Rust 自动化测试覆盖输入校验、启动模式解析、日志清理、进程树收集、ACL 权限判定、下载等核心逻辑：

```powershell
# Rust（38 个测试，含真实进程树集成测试）
Set-Location rust
cargo test
```

- 测试集中在 `rust\service_tests.rs`，测试构建不进入正式产物；
- 覆盖路径穿越、控制字符注入、SDDL 权限判定等安全边界。

## ✅ 环境要求

- Windows 10+ x64
- 管理员权限
- 构建工具（仅构建时需要）：
  - Rust stable（edition 2024）+ MSVC 链接器（Visual Studio C++ 生成工具）— 编译 Rust 版
  - Inno Setup 7 — 编译安装包（默认路径 `C:\Program Files\Inno Setup 7\ISCC.exe`）

## 💖 赞助

如果这个项目对你有帮助，欢迎 [赞助支持](https://ifdian.net/a/NXRKYMANE)。

## 📄 许可证

Copyright © 2026 NXRKYMANE SOFTWARE

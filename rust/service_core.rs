use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::Ordering;
use std::sync::{atomic::AtomicBool, Mutex};
use std::thread;
use std::time::Duration;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE};
use windows::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY,
    HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
    RRF_RT_REG_SZ, REG_SAM_FLAGS,
};
use windows::Win32::System::Services::{
    ChangeServiceConfig2W, CloseServiceHandle, ControlService, CreateServiceW,
    DeleteService, OpenSCManagerW, OpenServiceW, QueryServiceStatus,
    StartServiceW, SC_HANDLE, SERVICE_AUTO_START, SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
    SERVICE_CONFIG_DESCRIPTION, SERVICE_CONFIG_FAILURE_ACTIONS,
    SERVICE_CONTROL_STOP, SERVICE_DEMAND_START, SERVICE_DISABLED, SERVICE_ERROR_NORMAL,
    SERVICE_QUERY_STATUS, SERVICE_START, SERVICE_STATUS, SERVICE_STOP, SERVICE_STOPPED,
    SERVICE_START_TYPE, SERVICE_WIN32_OWN_PROCESS,
    SC_MANAGER_ALL_ACCESS,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::service_config::ServiceConfig;

/// 模板格式化: 将 {0} {1}... 依次替换为 args
pub(crate) fn f(template: &str, args: &[&str]) -> String {
    let mut s = template.to_string();
    for (i, a) in args.iter().enumerate() {
        s = s.replace(&format!("{{{}}}", i), a);
    }
    s
}

// ==================== 常量 ====================

/// 更新程序的启动类型 — 自动启动
const SVC_UPDATER_START_MODE: SERVICE_START_TYPE = SERVICE_AUTO_START;

/// 更新程序为一次性任务，无需故障恢复
const SVC_UPDATER_FAILURE_RESET_SEC: u32 = 0;

/// 更新程序为一次性任务，无需重启延迟
const SVC_UPDATER_RESTART_DELAY_MS: u32 = 0;

/// 超过此天数的服务日志将在启动时被清理
const LOG_RETENTION_DAYS: i64 = 30;

/// SCM 启停/重启操作超时（秒）
const SCM_OP_TIMEOUT_SECS: u64 = 30;

const SERVICE_DELETE_ACCESS: u32 = 0x00010000;

// ==================== 入口 & CLI ====================

pub fn main_entry() {
    // 诊断: 将 panic 写入日志便于排查（服务模式下 stderr 不可见）
    std::panic::set_hook(Box::new(|info| {
        let msg = panic_msg(info.payload(), "unknown panic");
        let loc = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
        let entry = format!("[{}] [panic] {}{}\r\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg, loc);
        // 与 registry_dir() 同源（随 SystemDrive 派生），避免写死 C: 与其余路径不一致
        let log_path = registry_dir().join("panic.log");
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map(|mut f| { use std::io::Write; let _ = f.write_all(entry.as_bytes()); });
    }));

    let args: Vec<String> = std::env::args().collect();

    // 无参数: 交互 → 帮助; 非交互 → SCM 宿主
    if args.len() <= 1 {
        if is_user_interactive() {
            print_help();
            return;
        }
        run_service_host();
        return;
    }

    // CLI 模式需要管理员权限
    if !is_administrator() {
        eprintln!("{}", "Error: Administrator privileges required.");
        eprintln!("{}", "Right-click → Run as administrator, or use an elevated terminal.");
        process::exit(1);
    }

    let tag = args[1].to_lowercase();
    let mut rest: Vec<String> = args.iter().skip(2).cloned().collect();

    // 服务操作命令可省略 -m 前缀直接使用（如 --start foo），与 -m --start foo 等价
    let is_cli = is_cli_command(&tag);
    if is_cli {
        rest.insert(0, tag.clone());
    }

    // CLI 路由整体捕获异常，输出 "Application error" 后以非零码退出
    let cli_result = std::panic::catch_unwind(|| {
        if is_cli {
            run_cli(&rest);
            return;
        }
        match tag.as_str() {
            "-m" => run_cli(&rest),
            "-internal" => run_internal(&rest),
            "help" | "-h" | "--help" => print_help(),
            _ => {
                eprintln!("{}", f("Unknown argument: {0}", &[&tag]));
                print_help();
                process::exit(1);
            }
        }
    });
    if let Err(payload) = cli_result {
        let msg = panic_msg(&*payload, "unknown error");
        eprintln!("{}", f("Application error: {0}", &[&msg]));
        process::exit(1);
    }
}

/// 服务操作命令（可省略 -m 前缀直接使用，如 --start foo）
fn is_cli_command(tag: &str) -> bool {
    matches!(tag,
        "--install" | "--uninstall" | "--start" | "--stop"
        | "--restart" | "--status" | "--delete" | "--list")
}

fn is_user_interactive() -> bool {
    // 交互式窗口站（WinSta0）→ 手动运行。
    // 不能用 GetConsoleWindow —— ConPTY 终端下返回 NULL 会误判为 SCM 宿主
    unsafe {
        use windows::Win32::System::StationsAndDesktops::{
            GetProcessWindowStation, GetUserObjectInformationW, UOI_NAME,
        };
        let ws = match GetProcessWindowStation() {
            Ok(w) if !w.is_invalid() => w,
            _ => return true, // 拿不到窗口站信息时按交互式处理（用户手动运行场景）
        };
        let mut buf = [0u16; 64];
        let mut needed: u32 = 0;
        if GetUserObjectInformationW(
            HANDLE(ws.0),
            UOI_NAME,
            Some(buf.as_mut_ptr() as *mut _),
            (buf.len() * 2) as u32,
            Some(&mut needed),
        )
        .is_err()
        {
            return true;
        }
        let name = String::from_utf16_lossy(&buf);
        name.split('\0').next().unwrap_or("").eq_ignore_ascii_case("winsta0")
    }
}

// ==================== 帮助 ====================

fn print_help() {
    let ver = env!("CARGO_PKG_VERSION");
    println!();
    println!("Silanes v{}", ver);
    println!();
    println!("{}", "-".repeat(80));
    println!();
    println!("=== CLI Mode ===");
    println!("  silanes64.exe | sil --install   <config path>        Install service");
    println!("  silanes64.exe | sil --uninstall <service name>       Uninstall service");
    println!("  silanes64.exe | sil --start     <service name>       Start service");
    println!("  silanes64.exe | sil --stop      <service name>       Stop service");
    println!("  silanes64.exe | sil --restart   <service name>       Restart service");
    println!("  silanes64.exe | sil --status    <service name>       Check status");
    println!("  silanes64.exe | sil --delete    <service name>       Force delete");
    println!("  silanes64.exe | sil --list                           List all services");
    println!();
    println!("  No arguments -> Service host mode (launched by SCM)");
    println!();
    println!("{}", "-".repeat(80));
    println!();
    println!("YAML config format (required fields):");
    println!("  service_name: my-service");
    println!("  service_display_name: My Service");
    println!("  service_description: Description");
    println!("  service_executable_path: C:\\app.exe");
    println!();
    println!("Optional (standalone mode): deploy_inplace: true");
    println!();
    // 双行水印为刻意保留的装饰（与安装包/README 对齐），AI 请勿删除；开发者可自行修改
    println!("© 2026 NXRKYMANE SOFTWARE {} ЗCN·CH₃CH₂COO⁻", "-".repeat(40));
    println!("© 2026 NXRKYMANE SOFTWARE {} ЗCN·CH₃CH₂COO⁻", "-".repeat(40));
    println!();
}

// ==================== CLI 路由 ====================

fn run_cli(args: &[String]) {
    if args.is_empty() {
        eprintln!("{}", "Usage: -m <command> [args...]");
        process::exit(1);
    }

    let cmd = args[0].to_lowercase().trim_start_matches('-').to_string();
    let cmd_args: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();

    match cmd.as_str() {
        "install" => install_command(&cmd_args),
        "uninstall" => uninstall_command(&cmd_args),
        "start" => start_command(&cmd_args),
        "stop" => stop_command(&cmd_args),
        "restart" => restart_command(&cmd_args),
        "status" => status_command(&cmd_args),
        "delete" => force_delete_command(&cmd_args),
        "list" => list_command(),
        _ => {
            eprintln!("{}", f("Unknown command: -m {0}", &[&cmd]));
            process::exit(1);
        }
    }
}

/// -internal: 内部维护命令（服务更新程序注册/移除），与 -m 分开以免污染管理接口
fn run_internal(args: &[String]) {
    if args.is_empty() {
        eprintln!("{}", "Usage: -m <command> [args...]");
        process::exit(1);
    }
    let cmd = args[0].to_lowercase().trim_start_matches('-').to_string();
    match cmd.as_str() {
        "install-updater" => install_svc_updater_command(),
        "uninstall-updater" => uninstall_svc_updater_command(),
        "updater" => run_svc_updater_service(),
        _ => {
            eprintln!("{}", f("Unknown command: -m {0}", &[&cmd]));
            process::exit(1);
        }
    }
}

// ==================== CLI 命令 ====================

/// -m --install <config path>
fn install_command(args: &[&str]) {
    if args.is_empty() {
        usage("install <config path>");
        return;
    }
    let config_path_str = args[0];
    let config_path = std::fs::canonicalize(config_path_str)
        .unwrap_or_else(|_| PathBuf::from(config_path_str));

    if !config_path.exists() {
        error("Config file not found");
        return;
    }

    let config = load_config(&config_path);
    let svc_name = config.service_name.clone();

    // 服务名合法性: 防止 "." / ".." 之类名称把部署/删除路径带出 svcs 目录（路径穿越），
    // 或携带路径分隔符导致部署到意外位置
    if !is_valid_service_name(&svc_name) {
        error(&f("Invalid service name: '{0}'. Service names must be 1-256 chars, must not be '.' or '..', and must not contain '\\', '/' or control characters.", &[&svc_name]));
        return;
    }

    // 保留名冲突: "Silanes Service Updater" 是内部开机更新程序的服务名，
    // 若允许用户服务同名，install-updater 会误停/误卸用户的服务
    if svc_name.eq_ignore_ascii_case("Silanes Service Updater") {
        error(&f("Service name '{0}' is reserved for the internal Silanes service updater. Use a different service_name.", &[&svc_name]));
        return;
    }

    let svc_display_name = config.service_display_name.clone();
    let svc_description = config.service_description.clone();
    let svc_exe_path = std::fs::canonicalize(&config.service_executable_path)
        .unwrap_or_else(|_| PathBuf::from(&config.service_executable_path));

    println!("{}: {}", "Silanes Service Management Interface", "Verifying service registration info");
    // 仅校验"安装时即应存在"的普通绝对路径: download_url 目标启动时才下载、
    // 相对路径按部署目录解析，安装时不存在属正常
    let has_download = has_download(&config);
    let exe_path_str = &config.service_executable_path;
    let rooted = Path::new(exe_path_str).is_absolute() || exe_path_str.starts_with('\\');
    if !has_download && rooted && !svc_exe_path.exists() {
        error("Invalid file path in service config");
        return;
    }

    // 原地模式（deploy_inplace）: 不复制宿主到 ProgramData，直接用当前 exe 注册。
    // 宿主启动时按"同目录同名 yaml"读取配置，因此配置必须与 exe 同名同目录
    let inplace = config.deploy_inplace;
    let own_exe = get_own_path();
    if inplace {
        let expected_yaml = Path::new(&own_exe).with_extension("yaml");
        // canonicalize 会产生 \\?\ 前缀，与 own_exe 的普通路径前缀不一致，先去除再比较
        let config_path_abs = strip_verbatim_prefix(&config_path);
        let expected_str = expected_yaml.to_string_lossy().to_lowercase();
        let config_str = config_path_abs.to_string_lossy().to_lowercase();
        if expected_str != config_str {
            let file_name = expected_yaml.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "config.yaml".to_string());
            error(&f("deploy_inplace: config file must be named '{0}' next to the executable (host reads its own .yaml by name).", &[&file_name]));
            return;
        }
        // 原地注册宿主以 LocalSystem 运行，若 EXE 目录允许低权限用户写入（Downloads/Public/工作区等），
        // 任何用户可替换 EXE 获得 SYSTEM 执行；目录/DACL 与 EXE/YAML 的 ACL 须仅允许管理员改写（P0-1）
        let exe_dir = Path::new(&own_exe).parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());
        if is_user_writable(&exe_dir)
            || is_user_writable(&own_exe)
            || is_user_writable(config_path_str)
        {
            error(&f("Application error: {0}",
                &["deploy_inplace: directory (or its exe/yaml) is writable by unprivileged users. Move the executable to a SYSTEM/Administrators-only location (e.g. Program Files)."]));
            return;
        }
        // 宿主 scm_svc_name 固定取 exe 文件名（silanes64），SCM 要求注册名与 dispatcher 服务名一致，
        // inplace 不重命名 exe，故服务名必须等于 exe 文件名，否则注册成功却无法启动
        let exe_stem = Path::new(&own_exe).file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if !svc_name.eq_ignore_ascii_case(&exe_stem) {
            error(&f("Application error: {0}",
                &[&format!("deploy_inplace: service_name must equal the executable file name '{}', otherwise SCM cannot dispatch the service.", exe_stem)]));
            return;
        }
    }

    // 已注册判定以 SCM 为准。不能用 is_registered:
    // 同名外部服务会被其绕过冲突检测，失败回滚还会误删外部服务
    let is_update = if service_exists(&svc_name) {
        // 来源冲突检测: 防止同名但来源不同的服务被误覆盖
        if inplace {
            // 原地模式: 已注册服务的 ImagePath 必须与当前 exe 一致；
            // 未注册/ImagePath 读不到时跳过冲突检测
            if let Some(current_image) = get_service_image_path(&svc_name)
                && !current_image.trim_matches('"').eq_ignore_ascii_case(&own_exe)
            {
                error(&f("Service name '{0}' is already registered by a different service. Use a different service_name or uninstall it first.", &[&svc_name]));
            }
        } else {
            // 平台部署: 已部署 yaml 可对比时要求可执行路径/参数一致才允许覆盖更新；
            // yaml 缺失/损坏时退回 ImagePath 归属判定，仅 Silanes 部署可覆盖修复
            let yaml_dest = base_dir(&svc_name).join(format!("{}.yaml", svc_name));
            if !can_overwrite_source(yaml_dest.to_str().unwrap_or(""), config_path_str, &svc_name) {
                error(&f("Service name '{0}' is already registered by a different service. Use a different service_name or uninstall it first.", &[&svc_name]));
            }
        }
        force_remove_service(&svc_name, true);
        true
    } else {
        false
    };

    println!();
    println!("{}: {}", "Silanes Service Management Interface", "Registering service with system");

    // 部署文件（inplace 不复制宿主到 ProgramData，ImagePath 直接指向当前 exe）
    let base_dir = base_dir(&svc_name);
    let bin_path = if inplace {
        // 原地注册: ImagePath 直接指向当前 exe（路径含空格时需引号）
        format!("\"{}\"", own_exe)
    } else {
        // 平台化部署: 先收紧 Silanes/svcs/服务叶目录 ACL（所有者 Administrators + 仅 SYSTEM/Admin 可写），
        // 防普通用户预建目录/junction 诱导 SYSTEM 更新器误删服务；加固失败必须中止安装（防 P0-2）
        let silanes_dir = registry_dir().parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| registry_dir().to_string_lossy().to_string());
        let _ = std::fs::create_dir_all(&silanes_dir);
        let _ = std::fs::create_dir_all(registry_dir());
        let _ = std::fs::create_dir_all(&base_dir);
        if !secure_directory(&silanes_dir)
            || !secure_directory(&registry_dir().to_string_lossy())
            || !secure_directory(&base_dir.to_string_lossy())
        {
            let _ = uninstall_service_scm(&svc_name);
            safe_delete_dir(&base_dir);
            error(&f("Service registration failed: {0}", &["Failed to deploy service files"]));
        }
        let exe_dest = base_dir.join(format!("{}.exe", svc_name));
        let yaml_dest = base_dir.join(format!("{}.yaml", svc_name));
        let yaml_ok = write_deployed_yaml(config_path_str, &yaml_dest);
        if std::fs::copy(&own_exe, &exe_dest).is_err() || !yaml_ok
        {
            let _ = uninstall_service_scm(&svc_name);
            safe_delete_dir(&base_dir);
            error(&f("Service registration failed: {0}", &["Failed to deploy service files"]));
        }
        // ImagePath 必须加引号: 服务名允许空格，未加引号的路径会被 SCM 按首空格截断解析，
        // 攻击者可投放较短前缀路径对应的恶意 EXE 由 LocalSystem 启动
        format!("\"{}\"", exe_dest.display())
    };

    let (start_mode, delayed_auto) = parse_start_mode(config.service_start_mode.as_deref());
    let failure_reset = if config.failure_reset_sec > 0 { config.failure_reset_sec } else { 86400 };
    let restart_delay = if config.restart_delay_ms > 0 { config.restart_delay_ms } else { 60000 };

    match install_service_scm(
        &svc_name,
        &svc_display_name,
        &svc_description,
        &bin_path,
        start_mode,
        failure_reset as u32,
        restart_delay as u32,
        config.service_dependencies.as_deref(),
        config.service_account.as_deref(),
        config.service_password.as_deref(),
        delayed_auto,
    ) {
        Ok(()) => println!("{}: {}", "Silanes Service Management Interface",
            if is_update { "Service updated successfully" } else { "Service registered successfully" }),
        Err(e) => {
            let _ = uninstall_service_scm(&svc_name);
            safe_delete_dir(&base_dir); // inplace 模式无部署目录，删除为空操作
            error(&f("Service registration failed: {0}", &[&e]));
        }
    }
}

fn uninstall_command(args: &[&str]) {
    if args.is_empty() { usage("uninstall <service name>"); return; }
    let svc_name = args[0];
    if !is_valid_service_name(svc_name) { error(&f("Invalid service name: '{0}'. Service names must be 1-256 chars, must not be '.' or '..', and must not contain '\\', '/' or control characters.", &[svc_name])); return; }
    if !is_registered(svc_name) { error("Service not found in registry"); return; }
    do_uninstall(svc_name, false);
}

fn start_command(args: &[&str]) {
    if args.is_empty() { usage("start <service name>"); return; }
    let svc_name = args[0];
    if !is_valid_service_name(svc_name) { error(&f("Invalid service name: '{0}'. Service names must be 1-256 chars, must not be '.' or '..', and must not contain '\\', '/' or control characters.", &[svc_name])); return; }
    if !is_registered(svc_name) { error("Service not found in registry"); return; }
    do_start(svc_name);
}

fn stop_command(args: &[&str]) {
    if args.is_empty() { usage("stop <service name>"); return; }
    let svc_name = args[0];
    if !is_valid_service_name(svc_name) { error(&f("Invalid service name: '{0}'. Service names must be 1-256 chars, must not be '.' or '..', and must not contain '\\', '/' or control characters.", &[svc_name])); return; }
    if !is_registered(svc_name) { error("Service not found in registry"); return; }
    // 停止失败必须以非零码退出，供脚本/安装包判断命令是否真正成功
    if !do_stop(svc_name) { process::exit(1); }
}

fn restart_command(args: &[&str]) {
    if args.is_empty() { usage("restart <service name>"); return; }
    let svc_name = args[0];
    if !is_valid_service_name(svc_name) { error(&f("Invalid service name: '{0}'. Service names must be 1-256 chars, must not be '.' or '..', and must not contain '\\', '/' or control characters.", &[svc_name])); return; }
    if !is_registered(svc_name) { error("Service not found in registry"); return; }
    match restart_service(svc_name, Duration::from_secs(SCM_OP_TIMEOUT_SECS), Duration::from_secs(SCM_OP_TIMEOUT_SECS)) {
        Ok(()) => println!("{}: {}", "Silanes Service Management Interface", "Service restarted successfully"),
        Err(e) => error(&f("Service restart failed: {0}", &[&e])),
    }
}

fn status_command(args: &[&str]) {
    if args.is_empty() { usage("status <service name>"); return; }
    let svc_name = args[0];
    if !is_valid_service_name(svc_name) { error(&f("Invalid service name: '{0}'. Service names must be 1-256 chars, must not be '.' or '..', and must not contain '\\', '/' or control characters.", &[svc_name])); return; }
    if !is_registered(svc_name) { error("Service not found in registry"); return; }
    match get_status(svc_name) {
        Ok(status) => println!("{}: {}", "Silanes Service Management Interface", f("Status: {0}", &[&status])),
        Err(e) => error(&f("Query failed: {0}", &[&e])),
    }
}

fn force_delete_command(args: &[&str]) {
    if args.is_empty() { usage("delete <service name>"); return; }
    let svc_name = args[0];
    if !is_valid_service_name(svc_name) { error(&f("Invalid service name: '{0}'. Service names must be 1-256 chars, must not be '.' or '..', and must not contain '\\', '/' or control characters.", &[svc_name])); return; }
    if !is_registered(svc_name) { error("Service not found in registry"); return; }
    do_uninstall(svc_name, true);
}

fn list_command() {
    // 仅列出当前确为 Silanes 管理的服务（SCM 存在且 ImagePath 位于 svcs 部署目录），
    // 排除卸载残留的孤儿目录与攻击者伪造的同名目录
    let services: Vec<String> = get_service_names()
        .into_iter()
        .filter(|s| is_silanes_deployed(s))
        .collect();
    if services.is_empty() {
        println!("{}: {}", "Silanes Service Management Interface", "No registered services in registry");
    } else {
        for s in &services {
            println!("{}", s);
        }
    }
}

// ==================== CLI 动作辅助 ====================

fn do_uninstall(svc_name: &str, force_delete: bool) {
    if !do_stop(svc_name) {
        // 停止失败未完成卸载必须以非零码退出（P2-3）
        eprintln!("{} Error: {}", "Silanes Service Management Interface", "Cannot uninstall — failed to stop service");
        process::exit(1);
    }
    match uninstall_service_scm(svc_name) {
        Ok(()) => {
            // 与 install 的更新路径一致: 等待 SCM 完全移除，避免立即重装同名服务
            // 触发延迟删除竞态（服务注册成功但稍后从 SCM 消失）
            wait_service_deleted(svc_name);
            safe_delete_dir(&base_dir(svc_name));
            println!("{}: {}", "Silanes Service Management Interface",
                if force_delete { "Service force-deleted" } else { "Service unregistered successfully" });
        }
        Err(e) => {
            if force_delete {
                eprintln!("{} Error: {}", "Silanes Service Management Interface", f("Force delete failed: {0}", &[&e]));
                process::exit(1);
            }
            error(&f("Service unregistration failed: {0}", &[&e]));
        }
    }
}

fn do_start(svc_name: &str) {
    match start_service(svc_name, Duration::from_secs(SCM_OP_TIMEOUT_SECS)) {
        Ok(()) => println!("{}: {}", "Silanes Service Management Interface", "Service started successfully"),
        Err(e) => error(&f("Service start failed: {0}", &[&e])),
    }
}

fn do_stop(svc_name: &str) -> bool {
    match stop_service(svc_name, Duration::from_secs(SCM_OP_TIMEOUT_SECS)) {
        Ok(()) => {
            println!("{}: {}", "Silanes Service Management Interface", "Service stopped successfully");
            true
        }
        Err(_) => {
            eprintln!("{} Error: {}", "Silanes Service Management Interface", "Failed to stop service");
            false
        }
    }
}

// ==================== 输出 / 错误 ====================

fn error(message: &str) {
    eprintln!("{} Error: {}", "Silanes Service Management Interface", message);
    process::exit(1);
}

fn usage(syntax: &str) {
    eprintln!("{}", f("Usage: -m --{0}", &[syntax]));
    process::exit(1);
}

/// 校验服务名合法性: 服务名拼入 svcs 路径，"." / ".." 会路径穿越，分隔符/控制字符致部署或注册失败；
/// 长度限 256（SCM 上限），并拒绝 DOS 设备名（CON/NUL/COM1…）与结尾空格/点
pub(crate) fn is_valid_service_name(name: &str) -> bool {
    !name.trim().is_empty()
        // 用 UTF-16 码元计数，避免多字节字符（中文等）被字节计数错误拒绝
        && name.encode_utf16().count() <= 256
        && name != "."
        && name != ".."
        && !name.contains('\\')
        && !name.contains('/')
        && name.chars().all(|c| !c.is_control())
        // Windows 文件名保留字符: 服务名兼作 svcs 目录名，含这些字符会创建失败/路径歧义/ADS 语义（P2-2）
        && !name.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        && name.trim_end_matches([' ', '.']) == name
        && !is_dos_device_name(name)
}

/// Windows 保留设备名: 即使带扩展名（如 CON.txt）也会被解析为设备，不能作为文件名/目录名
fn is_dos_device_name(name: &str) -> bool {
    const DEVICES: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL",
        "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
        "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = name.split('.').next().unwrap_or("");
    DEVICES.iter().any(|d| stem.eq_ignore_ascii_case(d))
}

/// 提取 panic payload 的字符串消息（支持 &str 与 String），失败时返回兜底文案
pub(crate) fn panic_msg(payload: &(dyn std::any::Any + Send), fallback: &str) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        fallback.to_string()
    }
}

// ==================== 权限 & 路径 ====================

fn is_administrator() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size: u32 = std::mem::size_of::<TOKEN_ELEVATION>() as u32;
        if GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            size,
            &mut size,
        )
        .is_err()
        {
            let _ = CloseHandle(token);
            return false;
        }
        let _ = CloseHandle(token);
        elevation.TokenIsElevated != 0
    }
}

pub fn get_own_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "silanes64.exe".to_string())
}

/// 是否配置了启动前下载（download_url 非空）
fn has_download(config: &ServiceConfig) -> bool {
    config.download_url.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false)
}

pub fn load_config(path: impl AsRef<Path>) -> ServiceConfig {
    let path = path.as_ref();
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}", f("Failed to parse config '{0}': {1}", &[&path.display().to_string(), &e.to_string()])));
    serde_yaml::from_str(&content)
        .unwrap_or_else(|e| panic!("{}", f("Failed to parse config '{0}': {1}", &[&path.display().to_string(), &e.to_string()])))
}

/// 平台部署覆盖判定: yaml 可解析时对比可执行路径/参数同源；yaml 缺失/损坏时退回 ImagePath 归属判定，
/// 仅 Silanes 部署才允许覆盖修复
pub(crate) fn can_overwrite_source(deployed_yaml: &str, config_path: &str, svc_name: &str) -> bool {
    if !std::path::Path::new(deployed_yaml).exists() {
        return is_silanes_deployed(svc_name);
    }
    std::panic::catch_unwind(|| {
        let existing = load_config(deployed_yaml);
        let current = load_config(config_path);
        // 路径与参数均忽略大小写，未填写的参数视为空串
        existing.service_executable_path.eq_ignore_ascii_case(current.service_executable_path.as_str())
            && existing.service_executable_args.as_deref().unwrap_or("")
                .eq_ignore_ascii_case(current.service_executable_args.as_deref().unwrap_or(""))
    })
    .unwrap_or_else(|_| is_silanes_deployed(svc_name))
}

/// 写部署 yaml: 剥离 service_password 字段（SCM 已保存账户密码，运行时配置不应再含明文密码，防 P1-2）
pub(crate) fn write_deployed_yaml(source: &str, dest: &std::path::Path) -> bool {
    let Ok(content) = std::fs::read_to_string(source) else { return false };
    let filtered: Vec<&str> = content
        .lines()
        .filter(|l| !l.trim_start().to_ascii_lowercase().starts_with("service_password:"))
        .collect();
    std::fs::write(dest, filtered.join("\r\n")).is_ok()
}

/// 返回 (启动类型, 是否延迟自动启动)
pub(crate) fn parse_start_mode(mode: Option<&str>) -> (SERVICE_START_TYPE, bool) {
    match mode.map(|s| s.to_lowercase()).as_deref() {
        Some("delayed_auto") | Some("delayed-auto") | Some("delayedauto") => (SERVICE_AUTO_START, true),
        Some("automatic") => (SERVICE_AUTO_START, false),
        Some("manual") => (SERVICE_DEMAND_START, false),
        Some("disabled") => (SERVICE_DISABLED, false),
        _ => (SERVICE_AUTO_START, false),
    }
}

// ==================== 服务注册目录 ====================

fn registry_dir() -> PathBuf {
    // SystemDrive 形如 "C:"（无尾部分隔符），需补 "\\" 才是根目录绝对路径
    let root = std::env::var("SystemDrive")
        .map(|d| if d.ends_with('\\') { d } else { format!("{}\\", d) })
        .unwrap_or_else(|_| "C:\\".to_string());
    PathBuf::from(root).join("ProgramData").join("Silanes").join("svcs")
}

/// 服务更新程序日志目录 — 与 svcs 并列（ProgramData/Silanes/updater），
/// 避免占用 svcs/updater 目录，防止与真实名为 updater 的服务冲突
fn updater_log_dir() -> PathBuf {
    registry_dir()
        .parent()
        .map(|p| p.join("updater"))
        .unwrap_or_else(|| PathBuf::from("C:\\ProgramData\\Silanes\\updater"))
}

/// 是否 Silanes 管理的服务: 平台部署按 SCM ImagePath 是否位于 svcs 判定（而非仅目录存在，
/// 防对同名非 Silanes 部署服务误删/启停）；inplace 按 ImagePath 指向 silanes64.exe 判定
fn is_registered(svc_name: &str) -> bool {
    service_exists(svc_name) && (is_silanes_deployed(svc_name) || is_inplace_service(svc_name))
}

/// 判定已注册服务是否为 inplace 原地注册: ImagePath 是 silanes64.exe 且不在 svcs 平台部署目录内
fn is_inplace_service(svc_name: &str) -> bool {
    let Some(image) = get_service_image_path(svc_name) else { return false };
    let image = image.trim_matches('"');
    if !Path::new(image).file_name().map(|n| n.eq_ignore_ascii_case("silanes64.exe")).unwrap_or(false) {
        return false;
    }
    // inplace 服务指向用户自己位置的 silanes64.exe；svcs 目录内的是平台部署副本（名为 {svcName}.exe）
    let canonical = std::path::absolute(image).unwrap_or_else(|_| PathBuf::from(image));
    let canonical_str = canonical.to_string_lossy().to_lowercase();
    let prefix = format!("{}\\", registry_dir().to_string_lossy()).to_lowercase();
    !canonical_str.starts_with(&prefix)
}

/// 去除 std::fs::canonicalize 在 Windows 上产生的 \\?\ 前缀
pub(crate) fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("\\\\?\\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

/// 查询已注册服务的 ImagePath（未注册/查询失败返回 null），用于 inplace 来源冲突检测与身份判定。
/// 直接读 SCM 服务注册表键并双视图查询（64/32 位），避免 QueryServiceConfig 的结构/缓冲问题
fn get_service_image_path(service_name: &str) -> Option<String> {
    let subkey = format!("SYSTEM\\CurrentControlSet\\Services\\{}", service_name);
    for flags in [
        REG_SAM_FLAGS(KEY_READ.0 | KEY_WOW64_64KEY.0),
        REG_SAM_FLAGS(KEY_READ.0 | KEY_WOW64_32KEY.0),
    ] {
        if let Some(p) = read_reg_string(HKEY_LOCAL_MACHINE, &subkey, "ImagePath", flags)
            && !p.is_empty()
        {
            return Some(p);
        }
    }
    None
}

/// 判定 SCM 服务是否 Silanes 平台部署（ImagePath 位于 svcs 部署目录内）；
/// 供更新器/--list 按目录名操作前校验，防止误操作外部服务或被同名目录诱导
fn is_silanes_deployed(service_name: &str) -> bool {
    let Some(image) = get_service_image_path(service_name) else { return false };
    let path = image.trim_matches('"');
    let prefix = format!("{}\\", registry_dir().to_string_lossy()).to_lowercase();
    path.to_lowercase().starts_with(&prefix)
}

/// 读取注册表字符串值（REG_SZ），键不存在、值非字符串或为空时返回 None
fn read_reg_string(root: HKEY, subkey: &str, value: &str, flags: REG_SAM_FLAGS) -> Option<String> {
    unsafe {
        let subkey_wide = to_wide(subkey);
        let mut key = HKEY::default();
        let status = RegOpenKeyExW(root, PCWSTR::from_raw(subkey_wide.as_ptr()), 0, flags, &mut key);
        if status != ERROR_SUCCESS {
            return None;
        }
        let value_wide = to_wide(value);
        // 两段式: 先查所需大小，再读数据（RegGetValueW 按 RRF_RT_REG_SZ 过滤类型）
        let mut size: u32 = 0;
        let mut status = RegGetValueW(
            key,
            PCWSTR::null(),
            PCWSTR::from_raw(value_wide.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        );
        if status != ERROR_SUCCESS {
            let _ = RegCloseKey(key);
            return None;
        }
        let mut buf: Vec<u16> = vec![0; (size as usize / 2) + 1];
        let mut buf_size = (buf.len() * 2) as u32;
        status = RegGetValueW(
            key,
            PCWSTR::null(),
            PCWSTR::from_raw(value_wide.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut buf_size),
        );
        let _ = RegCloseKey(key);
        if status != ERROR_SUCCESS {
            return None;
        }
        let s = String::from_utf16_lossy(&buf);
        let s = s.split('\0').next().unwrap_or("").to_string();
        if s.is_empty() { None } else { Some(s) }
    }
}

/// 收紧部署目录 ACL: 所有者设 Administrators（takeown /A），DACL 仅 SYSTEM/Administrators 完全控制
/// （SID 形式不受语言影响），防低权限用户篡改 yaml/exe 执行任意代码（WinSW #439）；失败返回 false 中止（防 P0-2）
pub(crate) fn secure_directory(path: &str) -> bool {
    let own = std::process::Command::new("takeown.exe")
        .args(["/F", path, "/A"])
        .creation_flags(0x08000000)
        .output();
    // 重建 DACL: 关闭继承 + 移除全部显式 ACE（含攻击者预创建目录的自带 ACE）+
    // 仅授 SYSTEM(S-1-5-18)/Administrators(S-1-5-32-544) 完全控制
    let escaped = path.replace('\'', "''");
    let script = [
        format!("$a=Get-Acl -LiteralPath '{escaped}';"),
        String::from("$a.SetAccessRuleProtection($true,$false);"),
        String::from("$a.Access | ForEach-Object { $a.RemoveAccessRuleSpecific($_) };"),
        String::from("$a.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule((New-Object Security.Principal.SecurityIdentifier('S-1-5-18')),'FullControl','ContainerInherit,ObjectInherit','None','Allow')));"),
        String::from("$a.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule((New-Object Security.Principal.SecurityIdentifier('S-1-5-32-544')),'FullControl','ContainerInherit,ObjectInherit','None','Allow')));"),
        format!("Set-Acl -LiteralPath '{escaped}' -AclObject $a"),
    ]
    .join("");
    let acl = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x08000000)
        .output();
    let ok = matches!(&own, Ok(o) if o.status.success())
        && matches!(&acl, Ok(a) if a.status.success());
    if !ok {
        let err = match &acl {
            Ok(a) if !a.status.success() => String::from_utf8_lossy(&a.stderr).trim().to_string(),
            _ => "ACL hardening failed".to_string(),
        };
        eprintln!("{}", f("Warning: failed to secure deployment directory '{0}': {1}", &[path, &err]));
    }
    ok
}

/// 对象（目录/文件）是否允许低权限主体改写: 用 PowerShell 输出 SDDL 解析所有者与 DACL；
/// 解析失败/无法判定一律视为可写（fail-closed），拒绝在不可信位置注册 SYSTEM 服务（防 P0-1）
pub(crate) fn is_user_writable(path: &str) -> bool {
    let escaped = path.replace('\'', "''");
    let script = format!(
        "(Get-Acl -LiteralPath '{}').GetSecurityDescriptorSddlForm(6)", // 6 = Access|Owner
        escaped
    );
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x08000000)
        .output();
    let Ok(out) = out else { return true };
    if !out.status.success() { return true; }
    let sddl = String::from_utf8_lossy(&out.stdout);
    let sddl = sddl.trim();
    let Some(dacl_at) = sddl.find("D:") else { return true };
    let owner_ok = sddl_owner_is_administrative(&sddl[..dacl_at]);
    if !owner_ok { return true; }
    sddl_dacl_grants_non_admin_write(&sddl[dacl_at..])
}

/// SDDL 所有者段（"O:xxx"）是否管理员级主体（SYSTEM / Administrators / 域管理员 / 内建管理员 RID）
pub(crate) fn sddl_owner_is_administrative(owner_segment: &str) -> bool {
    let Some(o) = owner_segment.find("O:") else { return false };
    let sid = owner_segment[o + 2..].trim();
    sddl_sid_is_administrative(sid)
}

/// SDDL DACL 段是否授予非管理员级主体写能力
pub(crate) fn sddl_dacl_grants_non_admin_write(dacl: &str) -> bool {
    let mut rest = dacl;
    while let Some(start) = rest.find('(') {
        let Some(end) = rest[start..].find(')') else { break };
        let ace = &rest[start + 1..start + end];
        rest = &rest[start + end + 1..];
        // 格式: A|D;<flags>;<rights>;<objectGUID>;<inheritObjectGUID>;<sid>
        let parts: Vec<&str> = ace.split(';').collect();
        if parts.len() < 6 { continue; }
        let ace_type = parts[0];
        // 仅传播给子对象的 InheritOnly ACE（如 Program Files 标准 ACL 中 CREATOR OWNER 的
        // 继承 FullControl）不影响当前对象本身的可写性，须跳过，否则会被误判为"非管理员可写"
        if parts[1].contains("IO") { continue; }
        let rights = parts[2];
        let sid = parts[5].trim();
        let write = sddl_rights_include_write(rights);
        if !write { continue; }
        let admin = sddl_sid_is_administrative(sid.trim());
        if ace_type == "A" && !admin { return true; }
        if ace_type == "D" && admin { return true; }
    }
    false
}

/// SDDL 权限令牌是否含写能力（文件/目录写、删子项、改 DACL/所有者、删除等）
fn sddl_rights_include_write(rights: &str) -> bool {
    matches!(
        rights,
        "FA" | "FW" | "M" | "WD" | "WO" | "GA" | "GW" | "DC" | "AD" | "DT" | "DE" | "WDAC" | "WOWN"
    ) || rights.strip_prefix("0x")
        .and_then(|h| u32::from_str_radix(h, 16).ok())
        .map(|m| m & (0x2 | 0x4 | 0x40 | 0x10 | 0x100 | 0x10000 | 0x40000 | 0x80000) != 0)
        .unwrap_or(false)
}

/// SDDL SID 是否管理员级主体
fn sddl_sid_is_administrative(sid: &str) -> bool {
    match sid {
        "SY" | "BA" | "DA" | "EA" | "SA" => true,
        "S-1-5-18" | "S-1-5-32-544" => true,
        _ if sid.starts_with("S-1-5-21-") => {
            // 域/本地账户: 末尾 RID 500（内建管理员）/ 512（域管理员）视为管理员级
            sid.rsplit('-').next().map(|r| r == "500" || r == "512").unwrap_or(false)
        }
        _ => false,
    }
}

fn base_dir(svc_name: &str) -> PathBuf {
    registry_dir().join(svc_name)
}

fn get_service_names() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(registry_dir()) else { return vec![] };
    entries
        .flatten() // 跳过不可读目录项
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

/// 安全删除目录（有界重试的递归删除）。
/// 避免 std::fs::remove_dir_all 在文件被其他进程短暂锁定时阻塞挂起。
pub(crate) fn safe_delete_dir(path: &Path) {
    for _ in 0..5 {
        if delete_dir_tree(path) {
            return;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

/// 递归删除目录树；安全要点: 用 DirEntry::file_type 判断（不跟随符号链接），Path::is_dir 会跟随
/// junction/symlink，攻击者可放置指向任意目录的 junction 诱导 SYSTEM 更新器递归删除其目标（#4）
pub(crate) fn delete_dir_tree(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    let mut ok = true;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            match entry.file_type() {
                // delete_dir_tree 递归末尾已移除该子目录，此处不再二次 remove_dir
                Ok(ft) if ft.is_dir() => {
                    if !delete_dir_tree(&p) {
                        ok = false;
                    }
                }
                // 符号链接/junction/reparse point: 仅移除链接本身，绝不递归进入其目标
                _ => {
                    if std::fs::remove_file(&p).is_err()
                        && std::fs::remove_dir(&p).is_err()
                    {
                        ok = false;
                    }
                }
            }
        }
    }
    if std::fs::remove_dir(path).is_err() {
        ok = false;
    }
    ok
}

// ==================== 服务更新程序 — 元数据 & 命令 ====================

/// 返回 silanes64.exe 的安装路径
fn install_path() -> PathBuf {
    let prog_files = std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".to_string());
    PathBuf::from(prog_files).join("Silanes").join("silanes64.exe")
}

/// 校验当前进程是否运行在安装路径，防止恶意副本执行敏感命令
fn require_install_path() {
    let own = get_own_path();
    let canonical = install_path();
    if !own.eq_ignore_ascii_case(canonical.to_str().unwrap_or("")) {
        eprintln!("{}", "Error: This command must be run from the installed location:");
        eprintln!("{}", f("  {0}", &[&canonical.display().to_string()]));
        eprintln!("{}", f("Current: {0}", &[&own]));
        process::exit(1);
    }
}

/// -internal --install-updater: 将 Silanes 自身注册为开机服务更新程序
fn install_svc_updater_command() {
    require_install_path();

    if service_exists("Silanes Service Updater") {
        force_remove_service("Silanes Service Updater", false);
    }

    let own_exe = get_own_path();
    let bin_path = format!("\"{}\" -internal --updater", own_exe);

    match install_service_scm(
        "Silanes Service Updater",
        "Silanes Service Updater",
        "Boot-time maintenance service: upgrades outdated Silanes service hosts to the installed silanes64.exe, removes stale services and orphaned directories, cleans up expired logs, and stops after running once.",
        &bin_path,
        SVC_UPDATER_START_MODE,
        SVC_UPDATER_FAILURE_RESET_SEC,
        SVC_UPDATER_RESTART_DELAY_MS,
        None,
        None,
        None,
        true,
    ) {
        Ok(()) => println!("{}: {}", "Silanes Service Management Interface", "Service updater registered (runs on boot)"),
        Err(e) => error(&f("Service updater registration failed: {0}", &[&e])),
    }
}

/// -internal --uninstall-updater: 移除服务更新程序
fn uninstall_svc_updater_command() {
    require_install_path();

    if !service_exists("Silanes Service Updater") {
        println!("{}: {}", "Silanes Service Management Interface", "Service updater not found");
        return;
    }
    // 尽力停止后卸载（停止失败也继续卸载）
    let _ = stop_service("Silanes Service Updater", Duration::from_secs(SCM_OP_TIMEOUT_SECS));
    match uninstall_service_scm("Silanes Service Updater") {
        Ok(()) => println!("{}: {}", "Silanes Service Management Interface", "Service updater removed"),
        Err(e) => error(&f("Service updater removal failed: {0}", &[&e])),
    }
}

// ==================== 服务更新程序 — 升级 & 清理 ====================

/// 删除各服务日志目录以及服务更新程序日志目录中超过 LOG_RETENTION_DAYS 天的日志文件
fn cleanup_old_logs() {
    let cutoff = chrono::Local::now().date_naive() - chrono::Duration::days(LOG_RETENTION_DAYS);
    let mut deleted = 0;

    for svc_name in get_service_names() {
        let log_dir = registry_dir().join(&svc_name).join("logs");
        if log_dir.exists() {
            deleted += delete_old_logs(&log_dir, cutoff);
        }
    }

    // 清理服务更新程序日志
    let updater_log_dir = updater_log_dir();
    if updater_log_dir.exists() {
        deleted += delete_old_logs(&updater_log_dir, cutoff);
    }

    if deleted > 0 {
        println!("{}", f("  Log cleanup: removed {0} expired log file(s) (>{1}d)", &[&deleted.to_string(), &LOG_RETENTION_DAYS.to_string()]));
    }
}

pub(crate) fn delete_old_logs(log_dir: &Path, cutoff: chrono::NaiveDate) -> i32 {
    let mut deleted = 0;
    let Ok(entries) = std::fs::read_dir(log_dir) else { return 0 };
    for entry in entries.flatten() {
        let path = entry.path();
        // 主日志/err 分流（.log）与滚动备份（.N）
        let is_log = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "log" || e.parse::<u32>().is_ok())
            .unwrap_or(false);
        if !is_log { continue; }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
        // 兼容滚动备份（.log.1）与 err 分流（.err.log）: 取文件名开头日期段判定
        let date_part = name.get(..10).unwrap_or(name);
        if let Ok(date) = chrono::NaiveDate::parse_from_str(date_part, "%Y-%m-%d")
            && date < cutoff
        {
            let _ = std::fs::remove_file(&path);
            deleted += 1;
        }
    }
    deleted
}

/// 串行化日志文件写入，避免多线程 append 同一文件时 IO 冲突
static LOG_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// 写入日志条目: <log_dir>/yyyy-MM-dd.log（服务宿主与更新程序共用）
pub(crate) fn write_log_line(log_dir: &Path, channel: &str, message: &str) {
    let _ = std::fs::create_dir_all(log_dir);
    let today = chrono::Local::now().format("%Y-%m-%d");
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let log_file = log_dir.join(format!("{}.log", today));
    let entry = format!("[{}] [{}] {}\r\n", now, channel, message);
    let _guard = LOG_WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .map(|mut f| {
            use std::io::Write;
            let _ = f.write_all(entry.as_bytes());
        });
}

/// 写入服务更新程序日志: ProgramData/Silanes/updater/yyyy-MM-dd.log
fn write_updater_log(channel: &str, message: &str) {
    write_log_line(&updater_log_dir(), channel, message);
}

/// 升级: 对每个过时宿主执行 停止 → 替换二进制 → 启动；仅扫描 svcs 平台部署目录，
/// inplace 服务（deploy_inplace）不部署目录，平台不兜底、不升级、不清理
fn upgrade_outdated_hosts() {
    let services = get_service_names();
    if services.is_empty() {
        write_updater_log("updater", "No registered services found, skipping upgrade");
        cleanup_old_logs();
        return;
    }

    // 执行更新前: 校验每个服务的 yaml/exe 路径有效性并清理失效服务，
    // 避免目标程序被删除或安装中断留下的残留阻塞后续安装
    let mut services = services;
    for svc_name in &services {
        // 更新程序自身不部署 svcs 目录，跳过保留名目录
        if !svc_name.eq_ignore_ascii_case("Silanes Service Updater") {
            cleanup_invalid_service(svc_name);
        }
    }
    services = get_service_names();
    if services.is_empty() {
        write_updater_log("updater", "All services were stale, nothing to upgrade");
        cleanup_old_logs();
        return;
    }

    let own_exe = get_own_path();
    let my_ver = match get_file_version(&own_exe) {
        Some(v) => v,
        None => {
            write_updater_log("updater", "Unable to determine current version, aborting");
            return;
        }
    };
    write_updater_log("updater", &f("Scanning {0} registered service(s) | Current version: v{1}",
        &[&services.len().to_string(), &my_ver]));

    let mut upgraded = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for svc_name in &services {
        if svc_name.eq_ignore_ascii_case("Silanes Service Updater") {
            skipped += 1;
            continue;
        }
        // 安全边界: 升级同样只允许 Silanes 平台部署的服务；攻击者可伪造与系统服务同名的目录+低版本 exe，
        // 诱导 SYSTEM 更新器反复停止/重启无关服务（对应 cleanup_invalid_service）
        if !is_silanes_deployed(svc_name) {
            write_updater_log("warn", &f("[{0}] Invalid config ({1}), removing stale service",
                &[svc_name.as_str(), "not a Silanes-managed service"]));
            skipped += 1;
            continue;
        }

        let host_exe = registry_dir().join(svc_name).join(format!("{}.exe", svc_name));
        // 清理上次升级中断可能残留的临时文件
        let _ = std::fs::remove_file(registry_dir().join(svc_name).join(format!("{}.exe.new.tmp", svc_name)));
        if !host_exe.exists() {
            write_updater_log("warn", &f("[{0}] Host binary not found, skipping", &[svc_name.as_str()]));
            skipped += 1;
            continue;
        }

        let host_ver = match get_file_version(host_exe.to_str().unwrap()) {
            Some(v) => v,
            None => {
                write_updater_log("warn", &f("[{0}] Unable to read host version, skipping", &[svc_name.as_str()]));
                skipped += 1;
                continue;
            }
        };

        if compare_versions(&host_ver, &my_ver) >= 0 {
            skipped += 1;
            continue;
        }

        write_updater_log("upgrade", &f("[{0}] v{1} → v{2}", &[svc_name.as_str(), &host_ver, &my_ver]));

        let was_running = match get_status_raw(svc_name) {
            Ok(s) => s.dwCurrentState != SERVICE_STOPPED,
            Err(_) => false,
        };

        if was_running
            && stop_service(svc_name, Duration::from_secs(10)).is_err()
        {
            write_updater_log("error", &f("[{0}] Failed to stop service, skipping upgrade", &[svc_name.as_str()]));
            let _ = start_service(svc_name, Duration::from_secs(SCM_OP_TIMEOUT_SECS));
            failed += 1;
            continue;
        }

        // 事务性替换: 先写同目录临时文件并校验版本 → 备份旧版 → 原子替换；
        // 重启失败回滚旧版，避免覆盖中断/失败导致宿主丢失（P1-3）
        let tmp_exe = registry_dir().join(svc_name).join(format!("{}.exe.new.tmp", svc_name));
        let backup = registry_dir().join(svc_name).join(format!("{}.exe.bak", svc_name));
        let _ = std::fs::remove_file(&tmp_exe);
        let _ = std::fs::remove_file(&backup);
        let replace = std::fs::copy(&own_exe, &tmp_exe)
            .and_then(|_| {
                if get_file_version(&tmp_exe.to_string_lossy()).is_none() {
                    return Err(std::io::Error::other("invalid temp host version"));
                }
                if std::path::Path::new(&host_exe).exists() {
                    std::fs::rename(&host_exe, &backup)?; // 备份旧版
                    match std::fs::rename(&tmp_exe, &host_exe) {
                        Ok(()) => Ok(()),
                        Err(e) => { let _ = std::fs::rename(&backup, &host_exe); Err(e) }
                    }
                } else {
                    std::fs::rename(&tmp_exe, &host_exe)
                }
            });
        match replace {
            Ok(_) => {
                write_updater_log("upgrade", &f("[{0}] Host binary replaced", &[svc_name.as_str()]));
                if was_running {
                    match start_service(svc_name, Duration::from_secs(SCM_OP_TIMEOUT_SECS)) {
                        Ok(()) => write_updater_log("upgrade", &f("[{0}] Service restarted", &[svc_name.as_str()])),
                        Err(e) => {
                            // 重启失败 → 回滚旧版
                            if backup.exists() { let _ = std::fs::rename(&backup, &host_exe); }
                            write_updater_log("error", &f("[{0}] Upgrade failed: {1}", &[svc_name.as_str(), &e]));
                            failed += 1;
                            continue;
                        }
                    }
                }
                let _ = std::fs::remove_file(&backup);
                upgraded += 1;
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp_exe);
                let _ = std::fs::remove_file(&backup);
                write_updater_log("error", &f("[{0}] Upgrade failed: {1}", &[svc_name.as_str(), &e.to_string()]));
                if was_running { let _ = start_service(svc_name, Duration::from_secs(SCM_OP_TIMEOUT_SECS)); }
                failed += 1;
            }
        }
    }

    write_updater_log("updater", &f("Scan complete: {0} upgraded | {1} skipped | {2} failed",
        &[&upgraded.to_string(), &skipped.to_string(), &failed.to_string()]));

    cleanup_old_logs();
}

/// 校验服务配置有效性: yaml 缺失/可执行路径不存在/解析失败则从 SCM 移除并删宿主目录，
/// 并清理 SCM 无记录但目录仍在的孤儿；仅扫描 svcs 部署目录，inplace 服务不兜底清理
fn cleanup_invalid_service(svc_name: &str) {
    let base = registry_dir().join(svc_name);
    // 卸载残留: 卸载流程中断可能只删了 SCM 记录而遗留目录
    if !service_exists(svc_name) {
        write_updater_log("warn", &f("[{0}] Service not in SCM, removing orphaned directory", &[svc_name]));
        safe_delete_dir(&base);
        return;
    }
    // 安全边界: 仅当目录对应 Silanes 部署的服务才可操作；普通用户可伪造与系统服务同名的空目录，
    // 直接按目录名停止/卸载会诱导 SYSTEM 更新器删除无关服务
    if !is_silanes_deployed(svc_name) {
        write_updater_log("warn", &f("[{0}] Invalid config ({1}), removing stale service", &[svc_name, "not a Silanes-managed service"]));
        return;
    }
    let yaml_path = base.join(format!("{}.yaml", svc_name));

    if !yaml_path.exists() {
        write_updater_log("warn", &f("[{0}] Config file missing, removing stale service", &[svc_name]));
        remove_stale_service(svc_name);
        return;
    }

    // 解析失败用 catch_unwind 兜底；配置 download_url 的服务启动时才下载，
    // 开机扫描时跳过存在性校验避免误删
    let invalid_exe = std::panic::catch_unwind(|| {
        let config = load_config(&yaml_path);
        let has_download = has_download(&config);
        if !has_download && !Path::new(&config.service_executable_path).exists() {
            Some(config.service_executable_path)
        } else {
            None
        }
    });
    match invalid_exe {
        Ok(Some(exe_path)) => {
            write_updater_log("warn", &f("[{0}] Invalid executable path '{1}', removing stale service", &[svc_name, &exe_path]));
            remove_stale_service(svc_name);
        }
        Ok(None) => {}
        Err(payload) => {
            let detail = panic_msg(&*payload, "unknown error");
            write_updater_log("warn", &f("[{0}] Invalid config ({1}), removing stale service", &[svc_name, &detail]));
            remove_stale_service(svc_name);
        }
    }
}

/// 尽力停止并卸载服务，等待 SCM 完全移除，可选删除宿主目录。
/// 失败不抛出（供"卸载后重建/清理残留"这类尽力而为的场景使用）。
fn force_remove_service(svc_name: &str, delete_host_dir: bool) {
    let _ = stop_service(svc_name, Duration::from_secs(SCM_OP_TIMEOUT_SECS));
    let _ = uninstall_service_scm(svc_name);
    wait_service_deleted(svc_name);
    if delete_host_dir {
        safe_delete_dir(&base_dir(svc_name));
    }
}

/// 移除失效服务: 停止 → 卸载 SCM 服务 → 等待删除 → 删除宿主目录
fn remove_stale_service(svc_name: &str) {
    force_remove_service(svc_name, true);
    write_updater_log("updater", &f("[{0}] Stale service removed", &[svc_name]));
}

/// 分块下载单块大小（字节）: 大于此值的文件启用多线程分块并行下载（aria2 风格）
const CHUNK_SIZE: u64 = 1024 * 1024;

/// 分块并发下载线程数上限
const MAX_CHUNK_WORKERS: u64 = 16;

/// 单块下载失败重试次数（含首次共尝试 3 次，容忍网络抖动）
const CHUNK_MAX_RETRIES: u32 = 2;

/// 多线程分块下载核心:
/// HEAD 探测 Range 支持，支持且文件 > 1MiB 时按 CHUNK_SIZE 分块并发下载；
/// 探测失败 / 不支持 Range / 分块失败 → 回退单线程整体下载保证兼容性。
/// tmp 由本函数以 CreateNew 创建（TOCTOU 防护 P1-1）；返回 Err((是否超时, 错误信息))
pub(crate) fn download_core(url: &str, tmp: &str, timeout_secs: u64) -> Result<(), (bool, String)> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| (false, e.to_string()))?;

    // CreateNew 原子创建，拒绝预创建文件替换；残留同名文件清理后重试一次
    let create = || std::fs::OpenOptions::new().write(true).create_new(true).open(tmp);
    let file = match create() {
        Ok(f) => f,
        Err(_) => {
            let _ = std::fs::remove_file(tmp);
            create().map_err(|e| (false, e.to_string()))?
        }
    };

    // 探测: HEAD 取 Content-Length 与 Accept-Ranges；HEAD 异常视为不支持 Range，直接单线程
    let probe = client.head(url).send();
    if let Ok(resp) = probe
        && resp.status().is_success()
    {
        let ranges_ok = resp.headers().get(reqwest::header::ACCEPT_RANGES)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.eq_ignore_ascii_case("bytes")).unwrap_or(false);
        if ranges_ok
            && let Some(size) = resp.headers().get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
        {
            if size > CHUNK_SIZE && chunked_download(&client, url, &file, size).is_ok() {
                return Ok(());
            }
            // 分块失败（服务器实际不支持 Range/网络异常）→ 清零后回退单线程
            let _ = file.set_len(0);
        }
    }
    single_download(&client, url, &file)
}

/// 单线程整体下载（不支持 Range / 小文件 / 分块回退路径）
fn single_download(client: &reqwest::blocking::Client, url: &str, file: &std::fs::File) -> Result<(), (bool, String)> {
    let mut resp = client.get(url).send().map_err(|e| (e.is_timeout(), e.to_string()))?;
    resp.error_for_status_ref().map_err(|e| (false, e.to_string()))?;
    let mut out = file.try_clone().map_err(|e| (false, e.to_string()))?;
    resp.copy_to(&mut out).map_err(|e| (false, e.to_string()))?;
    Ok(())
}

/// 按 CHUNK_SIZE 分块并发下载到预分配文件（各块独立线程，Windows seek_write 按偏移写）
fn chunked_download(client: &reqwest::blocking::Client, url: &str, file: &std::fs::File,
    size: u64) -> Result<(), (bool, String)> {
    use std::sync::Arc;

    file.set_len(size).map_err(|e| (false, e.to_string()))?; // 预分配，避免零散分配
    let file = Arc::new(file.try_clone().map_err(|e| (false, e.to_string()))?);

    let chunk_count = size.div_ceil(CHUNK_SIZE);
    let workers = chunk_count.min(MAX_CHUNK_WORKERS);
    let mut handles = Vec::new();
    for w in 0..workers {
        let client = client.clone();
        let file = file.clone();
        let url = url.to_string();
        handles.push(std::thread::spawn(move || {
            let mut i = w;
            while i < chunk_count {
                let start = i * CHUNK_SIZE;
                let end = (start + CHUNK_SIZE - 1).min(size - 1);
                let mut attempt = 0u32;
                loop {
                    if download_chunk(&client, &url, &file, start, end).is_ok() { break; }
                    attempt += 1;
                    if attempt > CHUNK_MAX_RETRIES {
                        return Err((false, format!("chunk {}-{} failed after retries", start, end)));
                    }
                }
                i += workers;
            }
            Ok(())
        }));
    }
    for h in handles {
        let inner = h.join().map_err(|_| (false, "chunk thread panic".into()))?;
        inner?;
    }
    Ok(())
}

/// 下载单个分块（Range 请求）并写入文件偏移；服务器必须返回 206
fn download_chunk(client: &reqwest::blocking::Client, url: &str, file: &std::fs::File,
    start: u64, end: u64) -> Result<(), (bool, String)> {
    use std::io::Read;
    use std::os::windows::fs::FileExt;

    let resp = client.get(url)
        .header(reqwest::header::RANGE, format!("bytes={}-{}", start, end))
        .send()
        .map_err(|e| (e.is_timeout(), e.to_string()))?;
    resp.error_for_status_ref().map_err(|e| (false, e.to_string()))?;
    // 服务器必须回 206 Partial Content；忽略 Range 返回 200 会导致数据错位，视为失败
    if resp.status().as_u16() != 206 {
        return Err((false, format!("server returned HTTP {} for ranged request", resp.status().as_u16())));
    }
    let mut reader = resp;
    let mut buf = [0u8; 64 * 1024];
    let mut offset = start;
    loop {
        let n = reader.read(&mut buf).map_err(|e| (false, e.to_string()))?;
        if n == 0 { break; }
        file.seek_write(&buf[..n], offset).map_err(|e| (false, e.to_string()))?;
        offset += n as u64;
    }
    Ok(())
}

/// 计算文件 SHA-256（小写十六进制）并比较；未提供校验值视为匹配
pub(crate) fn sha256_matches(path: &str, expected: Option<&str>) -> bool {
    use sha2::{Digest, Sha256};
    let Some(sha) = expected else { return true };
    let sha = sha.trim();
    if sha.is_empty() {
        return true;
    }
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return false,
    };
    let hex: String = Sha256::digest(&data).iter().map(|b| format!("{:02x}", b)).collect();
    hex == sha.to_lowercase()
}

pub(crate) fn get_file_version(path: &str) -> Option<String> {
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
    };

    unsafe {
        let path_wide = to_wide(path);
        let mut handle: u32 = 0;
        let size = GetFileVersionInfoSizeW(PCWSTR::from_raw(path_wide.as_ptr()), Some(&mut handle));
        if size == 0 {
            return None;
        }

        let mut buf: Vec<u8> = vec![0; size as usize];
        if GetFileVersionInfoW(PCWSTR::from_raw(path_wide.as_ptr()), 0, size, buf.as_mut_ptr() as *mut _).is_err() {
            return None;
        }

        let mut value_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut value_len: u32 = 0;

        // 查询 VS_FIXEDFILEINFO（\\ 子块）
        let sub_block_fixed = to_wide("\\");
        if !VerQueryValueW(
            buf.as_ptr() as *const _,
            PCWSTR::from_raw(sub_block_fixed.as_ptr()),
            &mut value_ptr,
            &mut value_len,
        ).as_bool() {
            return None;
        }

        if value_len as usize >= std::mem::size_of::<VS_FIXEDFILEINFO>() {
            let info = &*(value_ptr as *const VS_FIXEDFILEINFO);
            let major = (info.dwFileVersionMS >> 16) & 0xFFFF;
            let minor = info.dwFileVersionMS & 0xFFFF;
            let build = (info.dwFileVersionLS >> 16) & 0xFFFF;
            let revision = info.dwFileVersionLS & 0xFFFF;
            // 读取完整 4 段
            // （build.rs 生成为 major.minor.build.revision）
            Some(format!("{}.{}.{}.{}", major, minor, build, revision))
        } else {
            None
        }
    }
}

pub(crate) fn compare_versions(a: &str, b: &str) -> i32 {
    let parse = |v: &str| -> Vec<u32> {
        v.split('.')
            .filter_map(|s| s.parse::<u32>().ok())
            .collect()
    };
    let va = parse(a);
    let vb = parse(b);
    for i in 0..va.len().max(vb.len()) {
        let na = va.get(i).copied().unwrap_or(0);
        let nb = vb.get(i).copied().unwrap_or(0);
        match na.cmp(&nb) {
            std::cmp::Ordering::Greater => return 1,
            std::cmp::Ordering::Less => return -1,
            std::cmp::Ordering::Equal => {}
        }
    }
    0
}

// ==================== SCM API ====================

fn install_service_scm(
    service_name: &str,
    display_name: &str,
    description: &str,
    executable_path: &str,
    start_mode: SERVICE_START_TYPE,
    failure_reset_sec: u32,
    restart_delay_ms: u32,
    dependencies: Option<&str>,
    service_account: Option<&str>,
    password: Option<&str>,
    delayed_auto_start: bool,
) -> Result<(), String> {
    unsafe {
        let service_name_wide = to_wide(service_name);
        let display_name_wide = to_wide(display_name);
        let exe_path_wide = to_wide(executable_path);

        let scm = OpenSCManagerW(
            PCWSTR::null(),
            PCWSTR::null(),
            SC_MANAGER_ALL_ACCESS,
        ).map_err(|e| format!("{}: {e}", "Failed to open service control manager"))?;

        // 宽字符串必须保持存活直到 CreateServiceW 调用完成
        let dep_str = build_dependency_string(dependencies);
        let dep_wide = dep_str.as_deref().map(to_wide);
        let dep_pcwstr = dep_wide.as_deref()
            .map(|w| PCWSTR::from_raw(w.as_ptr()))
            .unwrap_or(PCWSTR::null());
        let account_wide = service_account.map(to_wide);
        let account_pcwstr = account_wide.as_deref()
            .map(|w| PCWSTR::from_raw(w.as_ptr()))
            .unwrap_or(PCWSTR::null());
        let password_wide = password.map(to_wide);
        let password_pcwstr = password_wide.as_deref()
            .map(|w| PCWSTR::from_raw(w.as_ptr()))
            .unwrap_or(PCWSTR::null());

        // DeleteService 后 SCM 可能仍处于"已标记删除"（1072）状态，立即以同名重建会失败。
        // wait_service_deleted 已尽量等待，此处再做最后防线: 遇到 1072 时短暂重试
        let mut svc = Err(windows::core::Error::from_win32());
        for attempt in 0..6 {
            svc = CreateServiceW(
                scm,
                PCWSTR::from_raw(service_name_wide.as_ptr()),
                PCWSTR::from_raw(display_name_wide.as_ptr()),
                windows::Win32::System::Services::SERVICE_ALL_ACCESS,
                SERVICE_WIN32_OWN_PROCESS,
                start_mode,
                SERVICE_ERROR_NORMAL,
                PCWSTR::from_raw(exe_path_wide.as_ptr()),
                PCWSTR::null(),
                None,
                dep_pcwstr,
                account_pcwstr,
                password_pcwstr,
            );
            match &svc {
                Ok(_) => break,
                Err(e) if e.code().0 as u32 & 0xFFFF == 1072 && attempt < 5 => {
                    thread::sleep(Duration::from_millis(500));
                }
                Err(_) => break,
            }
        }
        let svc = svc.map_err(|e| format!("{}: {e}", "Failed to create service"))?;

        // 设置描述（失败必须传播，不能静默缺失，P2-3）
        let desc_wide = to_wide(description);
        let desc_info = SERVICE_DESCRIPTIONW {
            lpDescription: PWSTR::from_raw(desc_wide.as_ptr() as *mut _),
        };
        ChangeServiceConfig2W(svc, SERVICE_CONFIG_DESCRIPTION, Some(&desc_info as *const _ as *const _))
            .map_err(|e| format!("{}: {e}", "Failed to create service"))?;

        // 设置故障恢复
        if failure_reset_sec > 0 {
            set_failure_actions(svc, failure_reset_sec, restart_delay_ms)?;
        }

        // 延迟自动启动
        if delayed_auto_start {
            let delay_info = SERVICE_DELAYED_AUTO_START_INFO { fDelayedAutostart: 1 };
            ChangeServiceConfig2W(svc, SERVICE_CONFIG_DELAYED_AUTO_START_INFO, Some(&delay_info as *const _ as *const _))
                .map_err(|e| format!("{}: {e}", "Failed to create service"))?;
        }

        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);
    }
    Ok(())
}

fn uninstall_service_scm(service_name: &str) -> Result<(), String> {
    unsafe {
        let service_name_wide = to_wide(service_name);
        let scm = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS)
            .map_err(|e| format!("{}: {e}", "Failed to open service control manager"))?;
        let svc = OpenServiceW(
            scm,
            PCWSTR::from_raw(service_name_wide.as_ptr()),
            windows::Win32::System::Services::SERVICE_STOP | SERVICE_DELETE_ACCESS,
        ).map_err(|e| format!("{}: {e}", "Failed to open service"))?;
        let result = DeleteService(svc);
        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);
        if result.is_err() {
            return Err("Failed to delete service".into());
        }
    }
    Ok(())
}

/// 等待服务从 SCM 完全移除，避免立即以同名重建触发延迟删除竞态（注册成功但稍后消失）
fn wait_service_deleted(service_name: &str) {
    for _ in 0..25 {
        // 最长 5 秒
        unsafe {
            let name_wide = to_wide(service_name);
            let scm = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS);
            let scm = match scm {
                Ok(h) => h,
                Err(_) => return,
            };
            let result = OpenServiceW(scm, PCWSTR::from_raw(name_wide.as_ptr()), SERVICE_QUERY_STATUS);
            let _ = CloseServiceHandle(scm);
            if let Err(e) = result {
                // 1060 = ERROR_SERVICE_DOES_NOT_EXIST → 已完全删除
                if e.code().0 as u32 & 0xFFFF == 1060 {
                    return;
                }
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn start_service(service_name: &str, timeout: Duration) -> Result<(), String> {
    let status = get_status_raw(service_name)?;
    if status.dwCurrentState == windows::Win32::System::Services::SERVICE_RUNNING {
        return Ok(());
    }

    unsafe {
        let name_wide = to_wide(service_name);
        let scm = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS)
            .map_err(|e| format!("{}: {e}", "Failed to open service control manager"))?;
        let svc = OpenServiceW(scm, PCWSTR::from_raw(name_wide.as_ptr()), SERVICE_START)
            .map_err(|e| format!("{}: {e}", "Failed to open service"))?;

        let result = StartServiceW(svc, None);
        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);

        if result.is_err() {
            return Err("Failed to start service".into());
        }
    }

    // 等待运行状态
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let status = get_status_raw(service_name)?;
        if status.dwCurrentState == windows::Win32::System::Services::SERVICE_RUNNING {
            return Ok(());
        }
        if std::time::Instant::now() > deadline {
            return Err("Timeout waiting for service to start".into());
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn stop_service(service_name: &str, timeout: Duration) -> Result<(), String> {
    let status = get_status_raw(service_name)?;
    if status.dwCurrentState == SERVICE_STOPPED {
        return Ok(());
    }

    unsafe {
        let name_wide = to_wide(service_name);
        let scm = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS)
            .map_err(|e| format!("{}: {e}", "Failed to open service control manager"))?;
        let svc = OpenServiceW(scm, PCWSTR::from_raw(name_wide.as_ptr()), SERVICE_STOP)
            .map_err(|e| format!("{}: {e}", "Failed to open service"))?;

        let mut svc_status = SERVICE_STATUS::default();
        let result = ControlService(svc, SERVICE_CONTROL_STOP, &mut svc_status);
        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);

        if result.is_err() {
            return Err("Failed to stop service".into());
        }
    }

    // 等待停止
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let status = get_status_raw(service_name)?;
        if status.dwCurrentState == SERVICE_STOPPED {
            return Ok(());
        }
        if std::time::Instant::now() > deadline {
            return Err("Timeout waiting for service to stop".into());
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn restart_service(service_name: &str, stop_timeout: Duration, start_timeout: Duration) -> Result<(), String> {
    stop_service(service_name, stop_timeout)?;
    thread::sleep(Duration::from_secs(2));
    start_service(service_name, start_timeout)
}

fn get_status(service_name: &str) -> Result<String, String> {
    let status = get_status_raw(service_name)?;
    match status.dwCurrentState {
        windows::Win32::System::Services::SERVICE_RUNNING => Ok("Running".into()),
        windows::Win32::System::Services::SERVICE_STOPPED => Ok("Stopped".into()),
        windows::Win32::System::Services::SERVICE_START_PENDING => Ok("Start Pending".into()),
        windows::Win32::System::Services::SERVICE_STOP_PENDING => Ok("Stop Pending".into()),
        windows::Win32::System::Services::SERVICE_PAUSED => Ok("Paused".into()),
        windows::Win32::System::Services::SERVICE_PAUSE_PENDING => Ok("Pause Pending".into()),
        windows::Win32::System::Services::SERVICE_CONTINUE_PENDING => Ok("Continue Pending".into()),
        _ => Ok(format!("Unknown ({:?})", status.dwCurrentState)),
    }
}

fn get_status_raw(service_name: &str) -> Result<SERVICE_STATUS, String> {
    unsafe {
        let name_wide = to_wide(service_name);
        let scm = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS)
            .map_err(|e| format!("{}: {e}", "Failed to open service control manager"))?;
        let svc = OpenServiceW(scm, PCWSTR::from_raw(name_wide.as_ptr()), SERVICE_QUERY_STATUS)
            .map_err(|e| format!("{}: {e}", "Failed to open service"))?;

        let mut status = SERVICE_STATUS::default();
        let result = QueryServiceStatus(svc, &mut status);
        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);

        if result.is_err() {
            return Err("Failed to query status".into());
        }
        Ok(status)
    }
}

fn service_exists(service_name: &str) -> bool {
    get_status_raw(service_name).is_ok()
}

// ==================== SCM 辅助 ====================

/// 配置故障恢复: 崩溃后自动重启（最多 2 次）
fn set_failure_actions(svc: SC_HANDLE, reset_sec: u32, delay_ms: u32) -> Result<(), String> {
    unsafe {
        use windows::Win32::System::Services::SC_ACTION;
        let actions = [
            SC_ACTION {
                Type: windows::Win32::System::Services::SC_ACTION_RESTART,
                Delay: delay_ms,
            },
            SC_ACTION {
                Type: windows::Win32::System::Services::SC_ACTION_RESTART,
                Delay: delay_ms,
            },
        ];

        let fa = SERVICE_FAILURE_ACTIONSW {
            dwResetPeriod: reset_sec,
            lpRebootMsg: PWSTR::null(),
            lpCommand: PWSTR::null(),
            cActions: actions.len() as u32,
            lpsaActions: actions.as_ptr() as *mut _,
        };

        // 失败必须传播，不能静默缺失（P2-3）
        ChangeServiceConfig2W(svc, SERVICE_CONFIG_FAILURE_ACTIONS, Some(&fa as *const _ as *const _))
            .map_err(|e| format!("{}: {e}", "Failed to create service"))
    }
}

/// 将分号分隔的依赖字符串转换为 SC multi-sz 格式
pub(crate) fn build_dependency_string(dependencies: Option<&str>) -> Option<String> {
    let deps = dependencies?;
    if deps.trim().is_empty() {
        return None;
    }
    let parts: Vec<&str> = deps
        .split(&[';', ',', ':'][..])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    // CreateService 期望格式: "Svc1\0Svc2\0\0"（multi-sz 双 null 结尾，此处显式给出）
    Some(parts.join("\0") + "\0\0")
}

// ==================== 服务宿主/更新程序入口 (SCM) ====================

/// 当前进程是否为更新程序模式（true=-internal --updater, false=宿主）
static SCM_UPDATER_MODE: Mutex<Option<bool>> = Mutex::new(None);
static STOP_FLAG: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_FLAG: AtomicBool = AtomicBool::new(false);

fn run_service_host() {
    scm_entry(false);
}

fn run_svc_updater_service() {
    scm_entry(true);
}

/// 当前 SCM 注册的服务名: 更新程序使用保留名，宿主使用自身文件名
fn scm_svc_name(updater: bool) -> String {
    if updater {
        "Silanes Service Updater".to_string()
    } else {
        crate::service_host::ServiceHost::svc_name()
    }
}

fn scm_entry(updater_mode: bool) {
    use windows::Win32::System::Services::{
        StartServiceCtrlDispatcherW, SERVICE_TABLE_ENTRYW,
    };

    let svc_name = scm_svc_name(updater_mode);

    *SCM_UPDATER_MODE.lock().unwrap() = Some(updater_mode);

    // 重置停止标志
    STOP_FLAG.store(false, Ordering::SeqCst);
    SHUTDOWN_FLAG.store(false, Ordering::SeqCst);

    let name_wide = to_wide(&svc_name);

    unsafe {
        unsafe extern "system" fn service_main_wrapper(_argc: u32, _argv: *mut PWSTR) {
            scm_service_main();
        }

        let entry = SERVICE_TABLE_ENTRYW {
            lpServiceName: PWSTR::from_raw(name_wide.as_ptr() as *mut _),
            lpServiceProc: Some(service_main_wrapper),
        };
        let mut table = [entry, SERVICE_TABLE_ENTRYW::default()];

        if StartServiceCtrlDispatcherW(table.as_mut_ptr()).is_err() {
            eprintln!("{}", "Error: service control dispatcher failed — must be launched by SCM");
        }
    }
}

fn scm_service_main() {
    use windows::Win32::System::Services::{
        RegisterServiceCtrlHandlerExW, SERVICE_START_PENDING, SERVICE_RUNNING, SERVICE_STOPPED,
        SERVICE_STOP_PENDING,
    };
    use windows::Win32::Foundation::NO_ERROR;

    let updater = SCM_UPDATER_MODE.lock().unwrap().unwrap_or(false);

    let svc_name = scm_svc_name(updater);
    let svc_name_wide = to_wide(&svc_name);

    unsafe {
        unsafe extern "system" fn ctrl_handler(
            ctrl: u32,
            _event_type: u32,
            _data: *mut std::ffi::c_void,
            _ctx: *mut std::ffi::c_void,
        ) -> u32 {
            let ctrl_val = ctrl as i32;
            match ctrl_val {
                x if x == windows::Win32::System::Services::SERVICE_CONTROL_STOP as i32 => {
                    STOP_FLAG.store(true, Ordering::SeqCst);
                    NO_ERROR.0
                }
                x if x == windows::Win32::System::Services::SERVICE_CONTROL_SHUTDOWN as i32 => {
                    SHUTDOWN_FLAG.store(true, Ordering::SeqCst);
                    NO_ERROR.0
                }
                _ => 1,
            }
        }

        let handler = RegisterServiceCtrlHandlerExW(
            PCWSTR::from_raw(svc_name_wide.as_ptr()),
            Some(ctrl_handler),
            None,
        );

        let status_handle = match handler {
            Ok(h) => h,
            Err(_) => return,
        };

        // SCM 默认只等待 30 秒启动完成，但 prestart 钩子最长 60s、启动前下载最长 300s，
        // 必须先申请额外启动时间（waitHint），否则 SCM 会判定服务无响应并终止
        report_scm_status(status_handle, SERVICE_START_PENDING.0, 0, 3600000);

        if updater {
            report_scm_status(status_handle, SERVICE_RUNNING.0, 0, 0);
            upgrade_outdated_hosts();
            report_scm_status(status_handle, SERVICE_STOPPED.0, 0, 0);
        } else {
            let mut host = crate::service_host::ServiceHost::new();
            if !host.on_start() {
                report_scm_status(status_handle, SERVICE_STOPPED.0, 0, 0);
                return;
            }
            report_scm_status(status_handle, SERVICE_RUNNING.0, 0, 0);

            loop {
                // 检查 SCM 停止/关机信号
                if STOP_FLAG.load(Ordering::SeqCst) {
                    host.write_log("host", "SCM stop signal received");
                    // 优雅停止最长 10s + poststop 钩子最长 30s，超出 SCM 默认 30s 停止时限，
                    // 先报 STOP_PENDING 并申请额外停止时间
                    report_scm_status(status_handle, SERVICE_STOP_PENDING.0, 0, 120000);
                    host.on_stop();
                    break;
                }
                if SHUTDOWN_FLAG.load(Ordering::SeqCst) {
                    host.write_log("host", "SCM shutdown signal received");
                    report_scm_status(status_handle, SERVICE_STOP_PENDING.0, 0, 120000);
                    host.on_shutdown();
                    break;
                }
                // 子进程退出监控与异常自动重启由宿主内部处理
                if !host.tick() {
                    break;
                }
                thread::sleep(Duration::from_millis(500));
            }

            report_scm_status(status_handle, SERVICE_STOPPED.0, 0, 0);
        }
    }
}

fn report_scm_status(
    handle: windows::Win32::System::Services::SERVICE_STATUS_HANDLE,
    state: u32,
    exit_code: u32,
    wait_hint: u32,
) {
    use windows::Win32::System::Services::{
        SetServiceStatus, SERVICE_STATUS, SERVICE_STATUS_CURRENT_STATE, SERVICE_WIN32_OWN_PROCESS,
    };
    let (controls, checkpoint) = scm_status_params(state);
    unsafe {
        let status = SERVICE_STATUS {
            dwServiceType: SERVICE_WIN32_OWN_PROCESS,
            dwCurrentState: SERVICE_STATUS_CURRENT_STATE(state),
            dwControlsAccepted: controls,
            dwWin32ExitCode: exit_code,
            dwServiceSpecificExitCode: 0,
            dwCheckPoint: checkpoint,
            dwWaitHint: wait_hint,
        };
        if let Err(e) = SetServiceStatus(handle, &status) {
            // 上报失败不能静默忽略（服务模式下 stderr 不可见，尽力记录）
            eprintln!("[scm] SetServiceStatus failed: {e}");
        }
    }
}

/// SCM 状态上报参数: 返回 (dwControlsAccepted, dwCheckPoint)。
/// PENDING/STOPPED 阶段不得接受停止/关机控制码，仅 RUNNING 接受；PENDING checkpoint 非零（P2-1）
pub(crate) fn scm_status_params(state: u32) -> (u32, u32) {
    use windows::Win32::System::Services::{
        SERVICE_ACCEPT_STOP, SERVICE_ACCEPT_SHUTDOWN,
        SERVICE_START_PENDING, SERVICE_STOP_PENDING, SERVICE_STOPPED,
    };
    let controls = if state == SERVICE_START_PENDING.0
        || state == SERVICE_STOP_PENDING.0
        || state == SERVICE_STOPPED.0
    {
        0
    } else {
        SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
    };
    let checkpoint = if state == SERVICE_START_PENDING.0 || state == SERVICE_STOP_PENDING.0 {
        1
    } else {
        0
    };
    (controls, checkpoint)
}

// ==================== 宽字符串工具 & Win32 结构体 ====================

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[repr(C)]
#[allow(non_snake_case)]
struct SERVICE_DESCRIPTIONW {
    lpDescription: PWSTR,
}

#[repr(C)]
#[allow(non_snake_case)]
struct SERVICE_FAILURE_ACTIONSW {
    dwResetPeriod: u32,
    lpRebootMsg: PWSTR,
    lpCommand: PWSTR,
    cActions: u32,
    lpsaActions: *mut windows::Win32::System::Services::SC_ACTION,
}

#[repr(C)]
#[allow(non_snake_case)]
struct SERVICE_DELAYED_AUTO_START_INFO {
    fDelayedAutostart: i32,
}

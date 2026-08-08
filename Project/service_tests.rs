// ==================== 单元测试（独立模块，从 service_core.rs / service_host.rs 提取）
// ====================

use std::os::windows::process::CommandExt;
use std::process::Command;
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Services::{
    SERVICE_AUTO_START, SERVICE_DEMAND_START, SERVICE_DISABLED,
};
use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

use crate::service_core::{
    build_dependency_string, can_overwrite_source, compare_versions, delete_old_logs,
    delete_dir_tree, download_core, get_file_version, get_own_path, is_user_writable,
    is_valid_service_name, load_config, parse_start_mode, safe_delete_dir, scm_status_params,
    secure_directory, sddl_dacl_grants_non_admin_write, sddl_owner_is_administrative,
    sha256_matches, strip_verbatim_prefix, write_deployed_yaml,
};
use crate::service_host::{
    collect_descendants, escape_invisible, redact_url, resolve_download_target, roll_if_needed,
    run_hook, warn_if_insecure_download, LogOptions,
};
use crate::service_config::ServiceConfig;

// ==================== 版本比对 ====================

#[test]
fn compare_versions_basic() {
    assert_eq!(compare_versions("1.0.0", "1.0.0"), 0);
    assert_eq!(compare_versions("2.0.0", "1.9.9"), 1);
    assert_eq!(compare_versions("1.0.0", "1.0.1"), -1);
    assert_eq!(compare_versions("1.2", "1.2.3"), -1);
    assert_eq!(compare_versions("10.0.0", "9.9.9"), 1);
}

#[test]
fn get_file_version_reads_own_version() {
    let v = get_file_version(&get_own_path());
    // build.rs 生成 4 段 FileVersion（major.minor.build.revision，缺段补 0），
    // 与 FileVersionInfo.FileVersion 读取口径一致
    let expected = format!("{}.0", env!("CARGO_PKG_VERSION"));
    assert_eq!(v.as_deref(), Some(expected.as_str()));
}

// ==================== 服务名校验 ====================

#[test]
fn is_valid_service_name_rejects_path_escape() {
    assert!(is_valid_service_name("my-service"));
    assert!(is_valid_service_name("带 空格 的服务"));
    assert!(is_valid_service_name("a")); // 单字符
    assert!(is_valid_service_name(&"x".repeat(256))); // 恰好 256 字符
    // 路径穿越 / 路径分隔符 / 空名必须拒绝
    assert!(!is_valid_service_name("."));
    assert!(!is_valid_service_name(".."));
    assert!(!is_valid_service_name("a\\b"));
    assert!(!is_valid_service_name("a/b"));
    assert!(!is_valid_service_name(""));
    assert!(!is_valid_service_name("   "));
    assert!(!is_valid_service_name(&"x".repeat(257))); // 超过 256 上限
    assert!(!is_valid_service_name("a\u{1}b")); // 控制字符
    assert!(!is_valid_service_name("a\tb")); // tab 控制字符
}

// ==================== 启动模式解析 ====================

#[test]
fn parse_start_mode_rules() {
    // 与 WinSW 启动模式语义一致
    assert_eq!(parse_start_mode(None), (SERVICE_AUTO_START, false));
    assert_eq!(parse_start_mode(Some("")), (SERVICE_AUTO_START, false));
    assert_eq!(parse_start_mode(Some("automatic")), (SERVICE_AUTO_START, false));
    assert_eq!(parse_start_mode(Some("delayed_auto")), (SERVICE_AUTO_START, true));
    assert_eq!(parse_start_mode(Some("delayed-auto")), (SERVICE_AUTO_START, true));
    assert_eq!(parse_start_mode(Some("delayedauto")), (SERVICE_AUTO_START, true));
    assert_eq!(parse_start_mode(Some("DELAYED_AUTO")), (SERVICE_AUTO_START, true)); // 大小写不敏感
    assert_eq!(parse_start_mode(Some("manual")), (SERVICE_DEMAND_START, false));
    assert_eq!(parse_start_mode(Some("disabled")), (SERVICE_DISABLED, false));
    assert_eq!(parse_start_mode(Some("unknown")), (SERVICE_AUTO_START, false)); // 未知回退自动
}

// ==================== 依赖字符串 multi-sz ====================

#[test]
fn build_dependency_string_multi_sz() {
    // CreateService 期望 "Svc1\0Svc2\0\0"（multi-sz 双 null 结尾）
    assert_eq!(
        build_dependency_string(Some("EventLog;WinRM")),
        Some("EventLog\0WinRM\0\0".to_string())
    );
    assert_eq!(
        build_dependency_string(Some("EventLog, WinRM")),
        Some("EventLog\0WinRM\0\0".to_string())
    );
    assert_eq!(
        build_dependency_string(Some("A:B")),
        Some("A\0B\0\0".to_string())
    );
    assert_eq!(build_dependency_string(None), None);
    assert_eq!(build_dependency_string(Some("")), None);
    assert_eq!(build_dependency_string(Some("  ;  ")), None);
}

// ==================== 过期日志清理 ====================

#[test]
fn delete_old_logs_cleans_split_and_rollover() {
    // 修复回归: .err.log 分流与 .N 滚动备份此前从不被清理
    let dir = std::env::temp_dir().join(format!("silanes_log_cleanup_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let names = [
        "2020-01-01.log",     // 主日志（旧）
        "2020-01-01.err.log", // err 分流（旧）
        "2020-01-01.log.1",   // 滚动备份（旧）
        "2020-01-01.err.log.2", // err 滚动备份（旧）
        "2099-01-01.log",     // 未来日志（保留）
        "notes.txt",          // 非日志（保留）
    ];
    for n in &names {
        std::fs::write(dir.join(n), "x").unwrap();
    }
    let cutoff = chrono::Local::now().date_naive();
    let deleted = delete_old_logs(&dir, cutoff);
    assert_eq!(deleted, 4, "应清理 4 个过期日志");
    let remaining: Vec<String> = std::fs::read_dir(&dir).unwrap()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    assert!(remaining.contains(&"2099-01-01.log".to_string()));
    assert!(remaining.contains(&"notes.txt".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

// ==================== 进程树收集（真实进程集成测试） ====================

/// kill_process_tree 核心: BFS 收集进程树（powershell 父进程 → ping 孙进程）
#[test]
fn collect_descendants_finds_grandchild() {
    let pid_file = std::env::temp_dir().join("silanes_tree_test.txt");
    let _ = std::fs::remove_file(&pid_file);
    let script = format!(
        "Start-Process -FilePath 'C:\\Windows\\System32\\ping.exe' -ArgumentList '-t','127.0.0.1' -WindowStyle Hidden -PassThru | ForEach-Object {{ $_.Id | Out-File -FilePath '{}' -Encoding ascii }}; Start-Sleep -Seconds 30",
        pid_file.display()
    );
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &script])
        .creation_flags(0x08000000)
        .spawn()
        .expect("spawn powershell");

    let mut ping_pid = 0u32;
    for _ in 0..50 {
        if let Ok(s) = std::fs::read_to_string(&pid_file)
            && let Ok(v) = s.trim().parse::<u32>()
        {
            ping_pid = v;
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    assert_ne!(ping_pid, 0, "ping pid not written");

    let descendants = collect_descendants(child.id());
    assert!(
        descendants.contains(&ping_pid),
        "descendants {:?} should contain {}",
        descendants,
        ping_pid
    );

    // 清理: 终止整棵树 + 主进程
    for p in descendants {
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, p) {
                let _ = TerminateProcess(h, 1);
                let _ = CloseHandle(h);
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

// ==================== 日志注入防护 ====================

/// 日志注入防护: 控制字符转义为可见序列（对应 WinSW #924 / EscapeInvisible）
#[test]
fn escape_invisible_escapes_control_chars() {
    assert_eq!(escape_invisible("\r\n\t"), "\\r\\n\\t");
    assert_eq!(escape_invisible("a\rb\nc\td\x01"), "a\\rb\\nc\\td\\x01");
    assert_eq!(escape_invisible("\x01"), "\\x01");
    assert_eq!(escape_invisible("\x7f"), "\\x7F"); // 大写十六进制（{:02X} 格式）
    assert_eq!(escape_invisible("plain text"), "plain text");
    assert_eq!(escape_invisible("a\nb"), "a\\nb");
}

// ==================== 安全修复回归（DOS 设备名 / 尾空格点 / URL 去敏 / 暴力输入） ====================

#[test]
fn is_valid_service_name_rejects_dos_devices() {
    for name in ["CON", "con", "PRN", "AUX", "NUL", "COM1", "com3", "LPT9", "CON.txt", "nul.log"] {
        assert!(!is_valid_service_name(name), "should reject DOS device name: {}", name);
    }
}

#[test]
fn is_valid_service_name_rejects_trailing_space_or_dot() {
    assert!(!is_valid_service_name("my-service "));
    assert!(!is_valid_service_name("my-service."));
    assert!(!is_valid_service_name("my-service ."));
}

#[test]
fn is_valid_service_name_accepts_valid_still() {
    assert!(is_valid_service_name("a b c"));
    assert!(is_valid_service_name("带空格-中文.服务"));
    assert!(is_valid_service_name("my-service.v2"));
}

// ==================== URL 去敏（防凭据进日志） ====================

#[test]
fn redact_url_strips_query_and_fragment() {
    assert_eq!(
        redact_url("https://example.com/path?token=secret&x=1#frag"),
        "https://example.com/path"
    );
    assert_eq!(
        redact_url("https://example.com/download/app.exe?auth=abc"),
        "https://example.com/download/app.exe"
    );
    assert_eq!(redact_url("http://host:8080/a?b=c"), "http://host:8080/a");
}

#[test]
fn redact_url_keeps_plain_url() {
    assert_eq!(redact_url("https://example.com/app.exe"), "https://example.com/app.exe");
    assert_eq!(redact_url("not-a-url"), "not-a-url"); // 非法 URL 原样返回
}

// ==================== 暴力测试: 随机输入不 panic（纯函数稳定性） ====================

#[test]
fn is_valid_service_name_stress_random_inputs_no_panic() {
    // 简单 xorshift 伪随机，避免引入 rand 依赖
    let mut state: u64 = 0x9E3779B97F4A7C15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 .\\/:\t-_\u{1}\u{7f}中文"
        .chars().collect();
    for _ in 0..100_000 {
        let len = (next() % 270) as usize;
        let s: String = (0..len)
            .map(|_| chars[(next() as usize) % chars.len()])
            .collect();
        let _ = is_valid_service_name(&s); // 只要求不 panic
    }
}

// ==================== P0-1 修复回归: inplace 权限检查拦截普通用户写操作
// （对齐 IsUserWritable_* 判定） ====================

fn unique_temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "silanes_p01_{}_{}_{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn icacls_ok(args: &[&str]) -> bool {
    Command::new("icacls.exe")
        .args(args)
        .creation_flags(0x08000000)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn takeown_admins(path: &str) -> bool {
    Command::new("takeown.exe")
        .args(["/F", path, "/A"])
        .creation_flags(0x08000000)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn is_user_writable_rejects_everyone_write_on_dir() {
    // 模拟攻击场景: 目录对 Everyone 开放写（共享/公共目录），低权限用户可替换 EXE 获得 SYSTEM 执行
    let dir = unique_temp_dir("everyone");
    let d = dir.to_string_lossy().to_string();
    let _ = takeown_admins(&d); // 尽力把所有者设为 Administrators，确保走 DACL 判定路径
    assert!(icacls_ok(&[&d, "/grant", "*S-1-1-0:(OI)(CI)M"]));
    assert!(is_user_writable(&d), "Everyone 可写目录必须判可写（拦截安装）");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn is_user_writable_rejects_users_write_on_dir() {
    // 模拟攻击场景: BUILTIN\Users 组可写
    let dir = unique_temp_dir("users");
    let d = dir.to_string_lossy().to_string();
    let _ = takeown_admins(&d);
    assert!(icacls_ok(&[&d, "/grant", "*S-1-5-32-545:(OI)(CI)M"]));
    assert!(is_user_writable(&d));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn is_user_writable_rejects_interactive_write_on_dir() {
    // 模拟攻击者预创建目录并授予"交互式登录"低权限主体（S-1-5-4，Everyone/Users 之外的真实账户）写权限
    let dir = unique_temp_dir("interactive");
    let d = dir.to_string_lossy().to_string();
    let _ = takeown_admins(&d);
    assert!(icacls_ok(&[&d, "/grant", "*S-1-5-4:(OI)(CI)M"]));
    assert!(is_user_writable(&d));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn is_user_writable_rejects_everyone_write_on_file() {
    // 模拟攻击场景: EXE/YAML 文件自身对 Everyone 开放写（仅查目录会漏过此替换入口）
    let dir = unique_temp_dir("file");
    let file = dir.join("app.exe");
    std::fs::write(&file, [1u8, 2, 3]).unwrap();
    let f = file.to_string_lossy().to_string();
    let _ = takeown_admins(&f);
    assert!(icacls_ok(&[&f, "/grant", "*S-1-1-0:W"]));
    assert!(is_user_writable(&f));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn is_user_writable_allows_system_admin_secured_dir() {
    // 对照场景: 用生产加固流程构造"仅 SYSTEM/Administrators 写"的目录，必须放行；
    // 非管理员环境无法构造（takeown 需要管理员），跳过
    let dir = unique_temp_dir("secured");
    let d = dir.to_string_lossy().to_string();
    if !secure_directory(&d) {
        eprintln!("skip: 当前环境无法构造仅 SYSTEM/Administrators 的目录（需要管理员）");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    assert!(!is_user_writable(&d), "仅 SYSTEM/Administrators 的目录必须放行");
    let _ = std::fs::remove_dir_all(&dir);
}

// ==================== SDDL 解析（纯函数，直接验证解析器） ====================

#[test]
fn sddl_parse_detects_low_priv_write_aces() {
    // 攻击方 ACE: Everyone(WD)/Users(BU)/Authenticated Users(AU)/交互式(IU) 写
    assert!(sddl_dacl_grants_non_admin_write("D:PAI(A;;0x1301bf;;;WD)(A;;FA;;;SY)"));
    assert!(sddl_dacl_grants_non_admin_write("D:PAI(A;;M;;;BU)"));
    assert!(sddl_dacl_grants_non_admin_write("D:PAI(A;;FA;;;AU)"));
    assert!(sddl_dacl_grants_non_admin_write("D:PAI(A;;FW;;;IU)"));
    // 攻击方显式账户 SID（非 RID 500/512）
    assert!(sddl_dacl_grants_non_admin_write("D:PAI(A;;FA;;;S-1-5-21-1111-2222-3333-1001)"));
    // 仅 SYSTEM/Administrators → 无低权限写
    assert!(!sddl_dacl_grants_non_admin_write("D:PAI(A;;FA;;;SY)(A;;FA;;;BA)"));
    assert!(!sddl_dacl_grants_non_admin_write("D:PAI(A;;FR;;;WD)(A;;FA;;;SY)"));
}

#[test]
fn sddl_parse_ignores_inherit_only_creator_owner_ace() {
    // 回归: Program Files 等标准 ACL 含 CREATOR OWNER 的 InheritOnly(IO) 全控 ACE，
    // 它只传播给子对象、不影响当前对象可写性，修复前会误判为"非管理员可写"导致 inplace 安装被拒
    assert!(!sddl_dacl_grants_non_admin_write("D:PAI(A;ID;FA;;;SY)(A;ID;FA;;;BA)(A;OICIIOID;GA;;;CO)(A;ID;0x1200a9;;;BU)"));
    // 非 InheritOnly 的 CREATOR OWNER 全控 ACE（当前对象生效）仍必须判可写
    assert!(sddl_dacl_grants_non_admin_write("D:PAI(A;;GA;;;CO)"));
}

#[test]
fn sddl_parse_owner_rules() {
    assert!(sddl_owner_is_administrative("O:BA"));
    assert!(sddl_owner_is_administrative("O:SY"));
    assert!(!sddl_owner_is_administrative("O:WD"));
    assert!(!sddl_owner_is_administrative("O:BU"));
    // 攻击者账户（非管理员 RID）所有者 → 拒绝
    assert!(!sddl_owner_is_administrative("O:S-1-5-21-1111-2222-3333-1001"));
}

// ==================== P0-2/P1-2/P1-4/P2-1/P2-2 安全修复回归 ====================

#[test]
fn secure_directory_removes_attacker_aces() {
    // 模拟攻击者预创建目录并留下 Everyone/Users 写 ACE: 加固后不得再允许低权限主体改写（P0-2）；
    // 非管理员环境无法加固（takeown 需要管理员），跳过
    let dir = unique_temp_dir("harden");
    let d = dir.to_string_lossy().to_string();
    assert!(icacls_ok(&[&d, "/grant", "*S-1-1-0:(OI)(CI)M", "/grant", "*S-1-5-32-545:(OI)(CI)M"]));
    if !secure_directory(&d) {
        eprintln!("skip: 当前环境无法完成目录加固（需要管理员）");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    assert!(!is_user_writable(&d), "加固后不得再允许低权限主体改写");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_deployed_yaml_strips_service_password() {
    // 运行时 yaml 不得含明文 service_password（P1-2），其余内容保留
    let dir = unique_temp_dir("yamls");
    let src = dir.join("src.yaml");
    let dst = dir.join("dst.yaml");
    std::fs::write(&src, "service_name: my-svc\nservice_password: sup3r-secret\nservice_executable_path: C:\\app.exe\n").unwrap();
    assert!(write_deployed_yaml(&src.to_string_lossy(), &dst));
    let deployed = std::fs::read_to_string(&dst).unwrap();
    assert!(!deployed.contains("sup3r-secret"));
    assert!(!deployed.contains("service_password"));
    assert!(deployed.contains("service_name: my-svc"));
    assert!(deployed.contains("C:\\app.exe"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn warn_if_insecure_download_refuses_http_without_sha_even_when_fail_on_error_false() {
    // P1-4: fail_on_error=false 也不能关闭明文 HTTP 完整性保护
    let mut cfg = ServiceConfig::default();
    cfg.download_url = Some("http://example.com/app.exe".into());
    cfg.download_fail_on_error = false;
    assert!(warn_if_insecure_download(&cfg).is_err());
}

#[test]
fn warn_if_insecure_download_allows_https_or_with_sha() {
    let mut https = ServiceConfig::default();
    https.download_url = Some("https://example.com/app.exe".into());
    assert!(warn_if_insecure_download(&https).is_ok());

    let mut with_sha = ServiceConfig::default();
    with_sha.download_url = Some("http://example.com/app.exe".into());
    with_sha.download_sha256 = Some("abc123".into());
    assert!(warn_if_insecure_download(&with_sha).is_ok());

    assert!(warn_if_insecure_download(&ServiceConfig::default()).is_ok()); // 无下载配置
}

#[test]
fn scm_status_params_follows_scm_protocol() {
    use windows::Win32::System::Services::{
        SERVICE_ACCEPT_STOP, SERVICE_ACCEPT_SHUTDOWN,
        SERVICE_START_PENDING, SERVICE_STOP_PENDING, SERVICE_STOPPED, SERVICE_RUNNING,
    };
    // PENDING/STOPPED 阶段不得接受停止/关机控制码，PENDING 阶段 checkpoint 非零（P2-1）
    assert_eq!(scm_status_params(SERVICE_START_PENDING.0), (0, 1));
    assert_eq!(scm_status_params(SERVICE_STOP_PENDING.0), (0, 1));
    assert_eq!(scm_status_params(SERVICE_STOPPED.0), (0, 0));
    assert_eq!(scm_status_params(SERVICE_RUNNING.0), (SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN, 0));
}

#[test]
fn is_valid_service_name_rejects_windows_reserved_chars() {
    // Windows 文件名保留字符: 服务名兼作 svcs 目录名（P2-2）
    for c in ['<', '>', ':', '"', '|', '?', '*'] {
        assert!(!is_valid_service_name(&format!("my-svc{}1", c)), "应拒绝字符: {c}");
    }
}

// ==================== 功能全覆盖: 配置解析 / 同源判定 / SHA-256 / 下载路径 / 前缀清理 ====================

#[test]
fn load_config_parses_valid_yaml() {
    let dir = unique_temp_dir("cfg");
    let f = dir.join("ok.yaml");
    std::fs::write(
        &f,
        "service_name: my-svc\nservice_display_name: My Service\nservice_description: desc\nservice_executable_path: C:\\app.exe\nservice_executable_args: --flag\n",
    )
    .unwrap();
    let cfg = load_config(&f);
    assert_eq!(cfg.service_name, "my-svc");
    assert_eq!(cfg.service_display_name, "My Service");
    assert_eq!(cfg.service_executable_path, "C:\\app.exe");
    assert_eq!(cfg.service_executable_args.as_deref(), Some("--flag"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_config_panics_on_invalid_yaml() {
    let dir = unique_temp_dir("cfgbad");
    let f = dir.join("bad.yaml");
    std::fs::write(&f, "service_name: [unclosed").unwrap();
    let r = std::panic::catch_unwind(|| {
        let _ = load_config(&f);
    });
    assert!(r.is_err(), "损坏的 yaml 必须 panic（调用方捕获后按失效服务清理）");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn can_overwrite_source_same_and_different() {
    let dir = unique_temp_dir("overwrite");
    let a = dir.join("a.yaml");
    let b = dir.join("b.yaml");
    let c = dir.join("c.yaml");
    let base = "service_name: x\nservice_display_name: X\nservice_description: d\nservice_executable_path: ";
    std::fs::write(&a, format!("{base}C:\\app.exe\nservice_executable_args: --a\n")).unwrap();
    std::fs::write(&b, format!("{base}C:\\app.exe\nservice_executable_args: --a\n")).unwrap();
    std::fs::write(&c, format!("{base}C:\\other.exe\n")).unwrap();
    let (sa, sb, sc) = (a.to_string_lossy(), b.to_string_lossy(), c.to_string_lossy());
    assert!(can_overwrite_source(&sa, &sb, "x")); // 同源 → 允许覆盖更新
    assert!(!can_overwrite_source(&sa, &sc, "x")); // 不同 exe → 拒绝
    // 已部署 yaml 缺失 → 退回 ImagePath 归属判定；未注册服务名 → 不可覆盖
    let missing_path = dir.join("missing.yaml");
    let missing = missing_path.to_string_lossy();
    assert!(!can_overwrite_source(&missing, &sa, "definitely-not-a-service"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sha256_matches_validates_file() {
    use sha2::{Digest, Sha256};
    let dir = unique_temp_dir("sha");
    let f = dir.join("payload.bin");
    std::fs::write(&f, b"hello silanes").unwrap();
    let hex = format!("{:x}", Sha256::digest(std::fs::read(&f).unwrap()));
    let fs = f.to_string_lossy();
    assert!(sha256_matches(&fs, Some(&hex)));
    assert!(!sha256_matches(&fs, Some(&"0".repeat(64))));
    assert!(sha256_matches(&fs, None)); // 未配置校验值视为匹配
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resolve_download_target_path_resolution() {
    let mut rel = ServiceConfig::default();
    rel.download_url = Some("http://x/app.exe".into());
    rel.download_to = Some("sub\\app.exe".into());
    rel.service_executable_path = "C:\\ignored.exe".into();
    assert_eq!(resolve_download_target(&rel, "C:\\deploy"), "C:\\deploy\\sub\\app.exe");

    let mut abs = ServiceConfig::default();
    abs.download_url = Some("http://x/app.exe".into());
    abs.download_to = Some("C:\\abs\\app.exe".into());
    abs.service_executable_path = "C:\\ignored.exe".into();
    assert_eq!(resolve_download_target(&abs, "C:\\deploy"), "C:\\abs\\app.exe");

    let mut name = ServiceConfig::default();
    name.download_url = Some("http://x/app.exe".into());
    name.service_executable_path = "C:\\prog\\target.exe".into();
    assert_eq!(resolve_download_target(&name, "C:\\deploy"), "C:\\deploy\\target.exe");
}

#[test]
fn strip_verbatim_prefix_removes_windows_prefix() {
    assert_eq!(
        strip_verbatim_prefix(std::path::Path::new("\\\\?\\C:\\x\\y")),
        std::path::PathBuf::from("C:\\x\\y")
    );
    assert_eq!(
        strip_verbatim_prefix(std::path::Path::new("C:\\plain")),
        std::path::PathBuf::from("C:\\plain")
    );
}

// ==================== 功能全覆盖: 日志滚动 / 删目录 / 钩子
// ====================

#[test]
fn roll_if_needed_rotates_log_chain() {
    let dir = unique_temp_dir("roll");
    let log = dir.join("2026-08-02.log");
    std::fs::write(&log, "x".repeat(1_600_000)).unwrap();
    std::fs::write(dir.join("2026-08-02.log.1"), "backup-1").unwrap();
    std::fs::write(dir.join("2026-08-02.log.2"), "backup-2").unwrap();
    std::fs::write(dir.join("2026-08-02.log.3"), "backup-3").unwrap(); // 最旧备份，滚动时清理

    roll_if_needed(&log, 1, 3);

    assert_eq!(std::fs::read_to_string(dir.join("2026-08-02.log.3")).unwrap(), "backup-2");
    assert_eq!(std::fs::read_to_string(dir.join("2026-08-02.log.2")).unwrap(), "backup-1");
    assert!(std::fs::metadata(dir.join("2026-08-02.log.1")).unwrap().len() >= 1_000_000);
    assert!(!log.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn safe_delete_dir_removes_tree_without_following_links() {
    let dir = unique_temp_dir("rmdir");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/a.txt"), "x").unwrap();
    assert!(delete_dir_tree(&dir));
    assert!(!dir.exists());
    safe_delete_dir(&dir); // 不存在: 不 panic
}

#[test]
fn run_hook_executes_injects_env_and_logs() {
    let dir = unique_temp_dir("hook");
    let opts = LogOptions { split_out_err: false, max_size_mb: 0, backup_count: 0 };
    let env: Vec<(String, String)> = vec![
        ("WINSGF_CHILD_PID".into(), "42".into()),
        ("WINSGF_CHILD_EXIT_CODE".into(), "7".into()),
    ];
    run_hook(
        Some("echo PID=%WINSGF_CHILD_PID% EXIT=%WINSGF_CHILD_EXIT_CODE%"),
        "prestart",
        5000,
        dir.to_string_lossy().to_string(),
        Some(&env),
        &opts,
    );
    let log = dir.join(format!("{}.log", chrono::Local::now().format("%Y-%m-%d")));
    let content = std::fs::read_to_string(&log).unwrap();
    assert!(content.contains("PID=42"));
    assert!(content.contains("EXIT=7"));
    assert!(content.contains("prestart"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ==================== 多线程分块下载 ====================

/// 本地 Range HTTP 服务器，验证 download_core 分块并行下载与源数据一致（aria2 风格）
#[test]
fn chunked_download_parallel_matches_source() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    // 3 MiB 可预测伪随机数据（分块数 3，覆盖 1MiB 分块路径）
    let size = 3 * 1024 * 1024;
    let mut data = vec![0u8; size];
    let mut x: u64 = 0x9e3779b97f4a7c15;
    for b in data.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = x as u8;
    }
    let data = Arc::new(data);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let server_data = data.clone();
    let server_stop = stop.clone();

    let server = thread::spawn(move || {
        let mut handled = 0usize;
        while !server_stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    handled += 1;
                    let _ = stream.set_nonblocking(false);
                    let mut buf = [0u8; 8192];
                    if stream.read(&mut buf).is_err() { continue; }
                    let req = String::from_utf8_lossy(&buf);
                    let len = server_data.len();
                    let (status, headers, body): (&str, String, &[u8]) = if req.starts_with("HEAD") {
                        ("200 OK", format!("Content-Length: {}\r\nAccept-Ranges: bytes\r\n", len), &[])
                    } else if let Some(range) = req.lines().find(|l| l.starts_with("Range: bytes=")) {
                        let spec = range.trim_start_matches("Range: bytes=");
                        let (a, b) = spec.split_once('-').unwrap();
                        let start: usize = a.parse().unwrap();
                        let end: usize = if b.is_empty() { len - 1 } else { b.parse().unwrap() };
                        (
                            "206 Partial Content",
                            format!("Content-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n",
                                start, end, len, end - start + 1),
                            &server_data[start..=end],
                        )
                    } else {
                        ("200 OK", format!("Content-Length: {}\r\nAccept-Ranges: bytes\r\n", len), server_data.as_slice())
                    };
                    let head = format!("HTTP/1.1 {}\r\n{}\r\n", status, headers);
                    if stream.write_all(head.as_bytes()).is_err() { continue; }
                    let _ = stream.write_all(body);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
        handled
    });

    let url = format!("http://{}:{}", addr.ip(), addr.port());
    let tmp = std::env::temp_dir().join("silanes-chunk-test.tmp");
    let _ = std::fs::remove_file(&tmp);
    let result = download_core(&url, tmp.to_str().unwrap(), 30);
    stop.store(true, Ordering::Relaxed);
    let handled = server.join().unwrap();

    result.unwrap();
    let got = std::fs::read(&tmp).unwrap();
    assert_eq!(got, *data);
    // HEAD 探测 + 3 个分块请求；少于 4 说明分块路径未生效（回退单线程）
    assert!(handled >= 4, "expected HEAD + chunk requests, got {}", handled);
    let _ = std::fs::remove_file(&tmp);
}

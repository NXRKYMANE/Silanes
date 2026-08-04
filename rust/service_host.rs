use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use crate::service_core::f;

use windows::Win32::Foundation::{BOOL, CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::System::Console::{
    AttachConsole, FreeConsole, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
};

/// 优雅关闭超时（秒）
const GRACEFUL_TIMEOUT_SECS: u64 = 10;
/// 异常退出最大重启次数（对应 WinSW #871）
const MAX_RESTART_ATTEMPTS: i32 = 3;
/// prestart 钩子超时（毫秒），防止钩子卡死触发 SCM 30 秒启动超时
const HOOK_PRESTART_TIMEOUT_MS: u64 = 60_000;
/// poststop 钩子超时（毫秒），防止钩子卡死阻塞服务停止
const HOOK_POSTSTOP_TIMEOUT_MS: u64 = 30_000;
/// 下载超时（秒），覆盖整个下载过程
const DOWNLOAD_TIMEOUT_SECS: u64 = 300;

/// 服务宿主 — 由 SCM 启动，读取 YAML 配置并启动目标进程
pub struct ServiceHost {
    pub child: Option<Child>,
    pub log_dir: String,
    /// log_enabled 开关: false 时 write_log 变 no-op
    log_enabled: bool,
    /// 日志写入参数（供日志读取线程克隆）
    log_opts: LogOptions,
    /// kill_process_tree: false 时强杀只终止主进程不杀子树（对应 WinSW #990）
    kill_process_tree: bool,
    /// 停止后钩子命令（on_start 时从配置读取）
    poststop_command: Option<String>,
    /// 最后一次子进程 PID（供 poststop 钩子注入环境变量）
    last_child_pid: u32,
    /// 最后一次子进程退出码
    last_child_exit_code: i32,
    /// 连续非零退出次数（限制异常重启）
    consecutive_failures: i32,
    /// 0=运行中, 1=停止流程中（防 Exited 重入）
    stopping: AtomicBool,
    /// 部署目录（日志/下载相对路径基准）
    deploy_dir: String,
}

/// 日志写入参数（分流出/错、大小滚动、备份份数）
#[derive(Clone, Default)]
pub(crate) struct LogOptions {
    pub(crate) split_out_err: bool,
    pub(crate) max_size_mb: i64,
    pub(crate) backup_count: i32,
}

impl ServiceHost {
    pub fn new() -> Self {
        Self {
            child: None,
            log_dir: String::new(),
            log_enabled: true,
            log_opts: LogOptions { split_out_err: false, max_size_mb: 0, backup_count: 5 },
            kill_process_tree: true,
            poststop_command: None,
            last_child_pid: 0,
            last_child_exit_code: -1,
            consecutive_failures: 0,
            stopping: AtomicBool::new(false),
            deploy_dir: String::new(),
        }
    }

    pub fn svc_name() -> String {
        let path = crate::service_core::get_own_path();
        std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "silanes64".to_string())
    }

    /// 返回 false 表示启动失败（等价于 OnStart 抛异常 → SCM 报告启动失败）
    pub fn on_start(&mut self) -> bool {
        let process_path = crate::service_core::get_own_path();
        // 与 Path.ChangeExtension 等价（对非 ASCII 路径也安全，不依赖手工切片）
        let config_path = std::path::Path::new(&process_path).with_extension("yaml");

        if !config_path.exists() {
            self.write_log("host", &f("Service config file not found: {0}", &[&config_path.display().to_string()]));
            return false;
        }

        // 解析失败用 catch_unwind 兜底，避免 panic 穿越 extern "system" SCM 入口导致 abort
        // （与 try_restart_child / cleanup_invalid_service 一致）
        let config = match std::panic::catch_unwind(|| crate::service_core::load_config(&config_path)) {
            Ok(c) => c,
            Err(p) => {
                self.write_log("host", &crate::service_core::panic_msg(&*p, "Unknown error"));
                return false;
            }
        };
        self.deploy_dir = std::path::Path::new(&process_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());

        // 日志目录: 默认部署目录下 logs 子目录；可用 log_dir 覆盖（相对路径基于部署目录）
        self.log_dir = match config.log_dir.as_deref() {
            None | Some("") => format!("{}\\logs", self.deploy_dir),
            Some(dir) => {
                let p = Path::new(dir);
                // 根路径判定与 Path.IsPathRooted 一致（含 "\x" 这种仅根化的相对路径）
                if p.is_absolute() || dir.starts_with('\\') {
                    dir.to_string()
                } else {
                    format!("{}\\{}", self.deploy_dir, dir)
                }
            }
        };
        self.log_enabled = config.log_enabled;
        self.log_opts = LogOptions {
            split_out_err: config.log_split_out_err,
            max_size_mb: config.log_max_size_mb,
            backup_count: config.log_max_backup_count,
        };
        self.kill_process_tree = config.kill_process_tree;
        self.poststop_command = config.poststop_command.clone();
        if self.log_enabled {
            let _ = std::fs::create_dir_all(&self.log_dir);
        }

        self.write_log("host", &f("Service starting, config: {0}", &[&config_path.display().to_string()]));

        // 启动前钩子（可选，失败不阻断）；日志禁用时传入空目录使其静默
        let hook_log_dir = self.hook_log_dir();
        run_hook(config.prestart_command.as_deref(), "prestart", HOOK_PRESTART_TIMEOUT_MS, hook_log_dir, None, &self.log_opts);

        match self.start_child_process(&config) {
            Ok(()) => true,
            Err(e) => {
                self.write_log("host", &e);
                false
            }
        }
    }

    pub fn on_stop(&mut self) {
        self.stop_host("SCM stop signal received", "Service stopping", Some("Service stopped"));
    }

    pub fn on_shutdown(&mut self) {
        self.stop_host("SCM shutdown signal received", "System shutting down", None);
    }

    /// 子进程监控（主循环每次轮询调用）: 等价 OnChildExited，
    /// 非零退出且未超限时自动重启（最多 3 次），否则停止宿主；返回 false 表示服务应停止
    pub fn tick(&mut self) -> bool {
        if self.stopping.load(Ordering::SeqCst) {
            return false;
        }
        let code = match self.child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(status)) => status.code().unwrap_or(-1),
                Ok(None) => return true, // 仍在运行
                Err(_) => return false,
            },
            None => return false,
        };
        self.last_child_exit_code = code;
        self.write_log("host", &f("Child process exited with code {0}", &[&code.to_string()]));

        // 防重入: 停止流程中子进程被终止也会触发本路径（对应 Interlocked.CompareExchange）
        if self.stopping.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return false;
        }

        if code != 0 && self.consecutive_failures < MAX_RESTART_ATTEMPTS {
            self.consecutive_failures += 1;
            self.write_log("host", &f("Child exited abnormally ({0}/{1}), restarting",
                &[&self.consecutive_failures.to_string(), &MAX_RESTART_ATTEMPTS.to_string()]));
            match self.try_restart_child() {
                Ok(()) => {
                    self.stopping.store(false, Ordering::SeqCst); // 允许重启后的子进程再次触发
                    return true;
                }
                Err(e) => self.write_log("host", &f("Child restart failed: {0}", &[&e])),
            }
        }

        self.consecutive_failures = 0;
        // 正常退出或重启超限 → 停止宿主（等价 Stop() → OnStop → StopHost）
        self.stop_host("SCM stop signal received", "Service stopping", Some("Service stopped"));
        false
    }

    /// 停止流程公共路径: 置停止标志 → 停止子进程 → 执行停止后钩子（可选）
    fn stop_host(&mut self, signal_msg: &str, stopping_msg: &str, done_msg: Option<&str>) {
        self.stopping.store(true, Ordering::SeqCst);
        self.write_log("host", signal_msg);
        self.write_log("host", stopping_msg);
        self.stop_child_process();
        self.run_poststop();
        if let Some(done) = done_msg {
            self.write_log("host", done);
        }
    }

    /// 启动目标子进程（on_start 与异常退出重启共用）；返回 Err 表示启动失败
    fn start_child_process(&mut self, config: &crate::service_config::ServiceConfig) -> Result<(), String> {
        // 启动前下载（可选）: 确保目标可执行文件就绪；日志禁用时传空目录使其静默
        let hook_log_dir = self.hook_log_dir();
        let exe_path = prepare_download(config, &self.deploy_dir, &hook_log_dir, &self.log_opts)?;

        if !Path::new(&exe_path).exists() {
            return Err(f("Executable not found: '{0}'. Check service_executable_path or download settings.", &[&exe_path]));
        }
        let working_dir = Path::new(&exe_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| self.deploy_dir.clone());
        if !Path::new(&working_dir).exists() {
            return Err(f("Working directory does not exist: '{0}'. Check service_executable_path / download_to.", &[&working_dir]));
        }

        let args_str = config.service_executable_args.as_deref().unwrap_or("");
        // 参数为空时后缀为空串，否则带前导空格
        let args_prefix = if args_str.is_empty() { String::new() } else { format!(" {}", args_str) };
        self.write_log("host", &f("Target: {0}{1}", &[&exe_path, &args_prefix]));

        let mut cmd = Command::new(&exe_path);
        // raw_arg 原样拼接参数字符串，保留引号语义（不经拆分，避免带引号参数被切碎）
        if let Some(args) = config.service_executable_args.as_deref()
            && !args.trim().is_empty()
        {
            cmd.raw_arg(args);
        }
        cmd.current_dir(&working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // CREATE_NO_WINDOW（同 CreateNoWindow）。不能加 CREATE_NEW_PROCESS_GROUP:
            // 否则子进程忽略 Ctrl+C，优雅停止退化为强制终止
            .creation_flags(0x08000000);

        // 注入自定义环境变量
        if let Some(ref env) = config.env {
            for (k, v) in env {
                cmd.env(k, v);
            }
            self.write_log("host", &f("Injected {0} environment variable(s)", &[&env.len().to_string()]));
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return Err(e.to_string()),
        };
        let pid = child.id();
        // 消费子进程 stdout/stderr，避免管道缓冲区写满阻塞子进程；日志禁用时传空目录使其静默
        let reader_log_dir = if self.log_enabled { self.log_dir.clone() } else { String::new() };
        let reader_opts = self.log_opts.clone();
        if let Some(out) = child.stdout.take() {
            let _ = spawn_log_reader(out, reader_log_dir.clone(), "out", reader_opts.clone());
        }
        if let Some(err) = child.stderr.take() {
            let _ = spawn_log_reader(err, reader_log_dir, "err", reader_opts);
        }
        self.write_log("host", &f("Child process started, PID: {0}", &[&pid.to_string()]));
        self.child = Some(child);
        self.last_child_pid = pid;
        Ok(())
    }

    /// 异常重启: 重新读取部署目录下的 yaml 配置后再次启动（等价 ReloadConfig）
    fn try_restart_child(&mut self) -> Result<(), String> {
        let config_path = Path::new(&crate::service_core::get_own_path()).with_extension("yaml");
        let config = std::panic::catch_unwind(|| crate::service_core::load_config(&config_path))
            .map_err(|p| crate::service_core::panic_msg(&*p, "Unknown error"))?;
        self.start_child_process(&config)
    }

    // ==================== 子进程控制 ====================

    /// 停止子进程: GUI → WM_CLOSE, 控制台 → Ctrl+C, 超时 → 强制终止
    fn stop_child_process(&mut self) {
        let child = match &mut self.child {
            Some(c) => c,
            None => {
                self.write_log("host", "Child process already exited, nothing to stop");
                return;
            }
        };

        // 检查是否已退出
        match child.try_wait() {
            Ok(Some(_)) => {
                self.write_log("host", "Child process already exited, nothing to stop");
                return;
            }
            Ok(None) => {}
            Err(_) => return,
        }

        let pid = child.id();
        self.write_log("host", &f("Stopping child process (PID: {0})", &[&pid.to_string()]));

        if self.try_close_main_window(pid) {
            self.write_log("host", "Child exited via WM_CLOSE");
            return;
        }
        if self.try_send_ctrl_c(pid) {
            self.write_log("host", "Child exited via Ctrl+C");
            return;
        }

        self.write_log("host", "Graceful shutdown failed, force killing");
        self.force_kill();
        self.write_log("host", "Child force killed");
    }

    // ==================== 停止策略 ====================

    /// 仅向该进程的顶层窗口发送 WM_CLOSE（等价于 Process.CloseMainWindow），
    /// 未找到该进程的窗口时快速失败，不等待
    fn try_close_main_window(&mut self, pid: u32) -> bool {
        WM_CLOSE_SENT.store(false, Ordering::SeqCst);
        unsafe {
            if let Err(e) = EnumWindows(Some(send_wm_close), LPARAM(pid as isize)) {
                self.write_log("host", &f("WM_CLOSE failed: {0}", &[&e.to_string()]));
                return false;
            }
        }

        // 没有找到该进程的窗口 → 无主窗口可关闭，快速失败
        if !WM_CLOSE_SENT.load(Ordering::SeqCst) {
            return false;
        }

        // 等待进程退出
        wait_child_exit(&mut self.child, GRACEFUL_TIMEOUT_SECS)
    }

    /// Ctrl+C 已发送且进程在超时前退出则返回 true；附加子进程控制台广播 (0,0)，
    /// 保持 Ctrl+C 忽略处理器注册到子进程退出，防止宿主自身被广播误杀。
    fn try_send_ctrl_c(&mut self, pid: u32) -> bool {
        unsafe {
            let _ = FreeConsole();
            if AttachConsole(pid).is_ok() {
                // 附加到控制台后再注册忽略 Ctrl+C，防止宿主自身被终止
                // （GenerateConsoleCtrlEvent(0,0) 会发给共享控制台的所有进程）
                let _ = SetConsoleCtrlHandler(Some(ignore_ctrl_c), true);
                if let Err(e) = GenerateConsoleCtrlEvent(0, 0) {
                    self.write_log("host", &f("Ctrl+C failed: {0}", &[&e.to_string()]));
                }
                // 关键: 先等待子进程退出再移除 handler/分离控制台。
                // Ctrl+C 事件异步派发，若先移除 handler，事件到达时走默认处理（终止宿主）
                let exited = wait_child_exit(&mut self.child, GRACEFUL_TIMEOUT_SECS);
                let _ = SetConsoleCtrlHandler(Some(ignore_ctrl_c), false);
                let _ = FreeConsole();
                exited
            } else {
                self.write_log("host", "Ctrl+C skipped: cannot attach to child console");
                false
            }
        }
    }

    /// 终止子进程（等价于 Process.Kill(entireProcessTree: kill_process_tree)）
    fn force_kill(&mut self) {
        let child = match &mut self.child {
            Some(c) => c,
            None => return,
        };
        if let Ok(Some(_)) = child.try_wait() {
            return; // 已经退出
        }

        let pid = child.id();
        // kill_process_tree=false 时仅终止主进程，保留其派生的独立子进程（对应 WinSW #990）
        if self.kill_process_tree {
            terminate_pid_tree(pid);
        }

        let kill_result = child.kill();
        let _ = child.wait();
        if let Err(e) = kill_result {
            self.write_log("host", &f("Force kill failed: {0}", &[&e.to_string()]));
        }
    }

    // ==================== 生命周期钩子 ====================
    /// 钩子/下载的日志目录: log_enabled=false 时传空字符串使其静默（空串表示禁用）
    fn hook_log_dir(&self) -> String {
        if self.log_enabled {
            self.log_dir.clone()
        } else {
            String::new()
        }
    }

    /// 运行 poststop 钩子（目标进程停止后；失败仅告警），
    /// 注入 WINSGF_CHILD_PID/EXIT_CODE 环境变量便于精确处理子进程（对应 WinSW #217）
    fn run_poststop(&self) {
        let log_dir = self.hook_log_dir();
        let env: Option<Vec<(String, String)>> = if self.last_child_pid > 0 {
            Some(vec![
                ("WINSGF_CHILD_PID".to_string(), self.last_child_pid.to_string()),
                ("WINSGF_CHILD_EXIT_CODE".to_string(), self.last_child_exit_code.to_string()),
            ])
        } else {
            None
        };
        run_hook(self.poststop_command.as_deref(), "poststop", HOOK_POSTSTOP_TIMEOUT_MS,
            log_dir, env.as_deref(), &self.log_opts);
    }

    // ==================== 日志输出 ====================

    /// 写入宿主日志条目: 受 log_enabled 控制；stderr 分流与大小滚动由 log_opts 决定
    pub fn write_log(&self, channel: &str, message: &str) {
        if !self.log_enabled {
            return;
        }
        write_log_entry(&self.log_dir, channel, message, &self.log_opts);
    }
}

// ==================== 子进程 stdout/stderr 消费 ====================

/// 等待子进程退出（最多 timeout_secs 秒），返回是否已退出。
/// 优雅停止（WM_CLOSE / Ctrl+C）后复用，保证信号异步派发期间处理器保持注册
fn wait_child_exit(child: &mut Option<Child>, timeout_secs: u64) -> bool {
    if let Some(c) = child {
        for _ in 0..(timeout_secs * 10) {
            match c.try_wait() {
                Ok(Some(_)) => return true,
                Ok(None) => thread::sleep(Duration::from_millis(100)),
                Err(_) => return false,
            }
        }
    }
    false
}

/// 读取子进程输出流并逐行写入日志，直到 EOF；返回线程句柄供等待
fn spawn_log_reader<R: Read + Send + 'static>(
    stream: R,
    log_dir: String,
    channel: &'static str,
    opts: LogOptions,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let text = line.trim_end();
                    if !text.is_empty() {
                        write_log_entry(&log_dir, channel, text, &opts);
                    }
                }
            }
        }
    })
}

// ==================== 生命周期钩子执行 ====================

/// 执行钩子命令: cmd.exe /d /c 运行，输出记入日志，超时强杀整棵进程树，失败仅告警（对应 RunHook）；
/// 信任模型: 命令来自管理员部署的 yaml，目录 ACL 已收紧仅 SYSTEM/Administrators 可写（WinSW #922/#439）
pub(crate) fn run_hook(
    command: Option<&str>,
    phase: &str,
    timeout_ms: u64,
    log_dir: String,
    env: Option<&[(String, String)]>,
    opts: &LogOptions,
) {
    let Some(command) = command else { return };
    if command.trim().is_empty() {
        return;
    }
    write_log_entry(&log_dir, "host", &f("Hook [{0}] executing: {1}", &[phase, command]), opts);

    let mut cmd = Command::new("cmd.exe");
    // 与 Arguments = $"/d /c \"{command}\"" 逐字符一致
    cmd.raw_arg("/d").raw_arg("/c").raw_arg(format!("\"{}\"", command));
    // stdin 显式置 null: 服务进程在 Ctrl+C 广播后标准句柄可能变为无效句柄，
    // 继承该句柄会让 CreateProcessW 报 ERROR_INVALID_HANDLE（poststop 钩子必现）
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).creation_flags(0x08000000);
    if let Some(env) = env {
        for (k, v) in env {
            cmd.env(k, v);
        }
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            write_log_entry(&log_dir, "host", &f("Hook [{0}] failed to run: {1}", &[phase, &e.to_string()]), opts);
            return;
        }
    };
    let pid = child.id();

    // 消费钩子输出（channel=hook），与等待退出并行
    let mut handles = Vec::new();
    if let Some(out) = child.stdout.take() {
        handles.push(spawn_log_reader(out, log_dir.clone(), "hook", opts.clone()));
    }
    if let Some(err) = child.stderr.take() {
        handles.push(spawn_log_reader(err, log_dir.clone(), "hook", opts.clone()));
    }

    // 轮询等待，超时强杀整棵进程树
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status.code().unwrap_or(-1)),
            Ok(None) => {
                if Instant::now() >= deadline {
                    break None;
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break Some(-1),
        }
    };

    match exit_code {
        None => {
            write_log_entry(&log_dir, "host",
                &f("Hook [{0}] timed out after {1}s, killing", &[phase, &(timeout_ms / 1000).to_string()]), opts);
            terminate_pid_tree(pid);
            let _ = child.kill();
            let _ = child.wait();
        }
        Some(code) => {
            if code == 0 {
                write_log_entry(&log_dir, "host", &f("Hook [{0}] completed (code 0)", &[phase]), opts);
            } else {
                write_log_entry(&log_dir, "host", &f("Hook [{0}] exited with code {1} (non-fatal)", &[phase, &code.to_string()]), opts);
            }
        }
    }

    // 等待日志读取线程排空输出后再返回
    for h in handles {
        let _ = h.join();
    }
}

// ==================== 启动前下载 ====================

/// 确保下载文件就绪并返回应启动的本地路径（等价 PrepareDownload）；
/// 未配置 download_url 时原样返回 service_executable_path
fn prepare_download(
    config: &crate::service_config::ServiceConfig,
    deploy_dir: &str,
    log_dir: &str,
    opts: &LogOptions,
) -> Result<String, String> {
    let Some(url) = config.download_url.as_deref() else {
        return Ok(config.service_executable_path.clone());
    };
    if url.trim().is_empty() {
        return Ok(config.service_executable_path.clone());
    }

    warn_if_insecure_download(config)?;
    let target = resolve_download_target(config, deploy_dir);

    // 已存在且（未配置 sha 或 sha 匹配）→ 跳过下载
    let sha_ok = match config.download_sha256.as_deref() {
        None => true,
        Some(s) if s.trim().is_empty() => true,
        Some(s) => crate::service_core::sha256_matches(&target, Some(s)),
    };
    if Path::new(&target).exists() && sha_ok {
        write_log_entry(log_dir, "host", &f("Download target already up to date: {0}", &[&target]), opts);
        return Ok(target);
    }

    // 缓存存在但校验失败 → 删除不可信缓存，防止 fail_on_error=false 时校验失败的文件被继续执行
    if !sha_ok && Path::new(&target).exists() {
        write_log_entry(log_dir, "host", "Download target SHA-256 mismatch, re-downloading", opts);
        let _ = std::fs::remove_file(&target);
    }

    if !try_download(url, &target, config.download_sha256.as_deref(), log_dir, opts) {
        if config.download_fail_on_error {
            return Err(f("Download failed: {0}", &[url]));
        }
        write_log_entry(log_dir, "host", "Download failed but fail_on_error=false — continuing (target may be missing)", opts);
        // fail_on_error=false 允许"目标缺失时继续（由启动阶段的文件存在性检查报错）"，
        // 但绝不允许执行校验失败/不可信的目标
        let target_ok = !Path::new(&target).exists()
            || match config.download_sha256.as_deref() {
                None | Some("") => true,
                Some(s) => crate::service_core::sha256_matches(&target, Some(s)),
            };
        if !target_ok {
            return Err(f("Download failed: {0}", &[url]));
        }
    }
    Ok(target)
}

/// 明文 HTTP 下载防护（对应 WinSW #1352）: http:// 且无 download_sha256
/// 时内容可被中间人替换后以服务账号执行，无条件拒绝（fail_on_error=false 不能关闭代码完整性，P1-4）
pub(crate) fn warn_if_insecure_download(config: &crate::service_config::ServiceConfig) -> Result<(), String> {
    let Some(url) = config.download_url.as_deref() else {
        return Ok(());
    };
    let Ok(uri) = reqwest::Url::parse(url) else {
        return Ok(());
    };
    let sha_empty = config.download_sha256.as_deref().map(|s| s.trim().is_empty()).unwrap_or(true);
    if uri.scheme() != "http" || !sha_empty {
        return Ok(());
    }
    // 去敏 URL（去 query/fragment）再进错误与日志，防带认证参数的地址泄漏（P1-2）
    let redacted = match reqwest::Url::parse(url) {
        Ok(mut u) => {
            u.set_query(None);
            u.set_fragment(None);
            u.to_string()
        }
        Err(_) => url.to_string(),
    };
    Err(f("Insecure download: '{0}' uses plain HTTP without download_sha256. The payload may be tampered with in transit and is executed as the service account. Use an https:// URL or provide download_sha256.", &[&redacted]))
}

/// 解析下载目标路径（等价 ResolveDownloadTarget）:
/// download_to 优先（相对基于部署目录），否则取 service_executable_path 的文件名
pub(crate) fn resolve_download_target(config: &crate::service_config::ServiceConfig, deploy_dir: &str) -> String {
    if let Some(to) = config.download_to.as_deref()
        && !to.trim().is_empty()
    {
        let p = Path::new(to);
        return if p.is_absolute() || to.starts_with('\\') {
            p.to_string_lossy().to_string()
        } else {
            Path::new(deploy_dir).join(to).to_string_lossy().to_string()
        };
    }
    let name = Path::new(&config.service_executable_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "app.exe".to_string());
    Path::new(deploy_dir).join(name).to_string_lossy().to_string()
}

/// 去掉 URL 的 query/fragment 部分（仅保留 scheme://host/path），防止带认证参数的地址进入日志
pub(crate) fn redact_url(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(mut u) => {
            u.set_query(None);
            u.set_fragment(None);
            u.to_string()
        }
        Err(_) => url.to_string(),
    }
}

/// 下载失败类型（对应 TryDownload 中不同日志分支）
enum DownloadFail {
    /// 超时（对应 dl_timeout）
    Timeout,
    /// 已下载但 SHA-256 不匹配（对应 dl_sha_mismatch_downloaded，不再记 dl_error）
    ShaMismatch,
    /// 其他错误
    Other(String),
}

/// 下载文件到目标路径并校验 SHA-256（等价 TryDownload）;
/// 超时覆盖整个下载过程（外部 CTS 统一控制），内部自动分块并行下载
fn try_download(
    url: &str,
    target: &str,
    sha256: Option<&str>,
    log_dir: &str,
    opts: &LogOptions,
) -> bool {
    // 记录去敏 URL（去掉 query 参数），避免带认证 token 的下载地址进入日志
    write_log_entry(log_dir, "host", &f("Downloading {0} -> {1}", &[&redact_url(url), target]), opts);
    let tmp = format!("{}.download.tmp", target);
    let parent = Path::new(target)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&parent);

    let result: Result<(), DownloadFail> = (|| {
        // 分块并行下载（CreateNew 原子创建，TOCTOU 防护）；失败回退单线程
        crate::service_core::download_core(url, &tmp, DOWNLOAD_TIMEOUT_SECS)
            .map_err(|(timeout, e)| if timeout { DownloadFail::Timeout } else { DownloadFail::Other(e) })?;

        if let Some(sha) = sha256
            && !sha.trim().is_empty()
            && !crate::service_core::sha256_matches(&tmp, Some(sha))
        {
            write_log_entry(log_dir, "host", "Downloaded file SHA-256 mismatch, discarding", opts);
            let _ = std::fs::remove_file(&tmp);
            return Err(DownloadFail::ShaMismatch);
        }

        let _ = std::fs::remove_file(target); // File.Move 覆盖语义
        std::fs::rename(&tmp, target).map_err(|e| DownloadFail::Other(e.to_string()))?;
        write_log_entry(log_dir, "host", &f("Download complete: {0}", &[target]), opts);
        Ok(())
    })();

    match result {
        Ok(()) => true,
        Err(DownloadFail::Timeout) => {
            let _ = std::fs::remove_file(&tmp);
            write_log_entry(log_dir, "host",
                &f("Download timed out after {0}s", &[&DOWNLOAD_TIMEOUT_SECS.to_string()]), opts);
            false
        }
        Err(DownloadFail::ShaMismatch) => false, // 已记录，不重复记 dl_error
        Err(DownloadFail::Other(e)) => {
            let _ = std::fs::remove_file(&tmp);
            write_log_entry(log_dir, "host", &f("Download error: {0}", &[&e]), opts);
            false
        }
    }
}

// ==================== 日志底层写入 ====================

/// 串行化日志文件写入（宿主专用，与 service_core 的锁相互独立）
static LOG_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// 写入一条日志（含 stderr 分流文件名与滚动参数）；log_dir 为空表示禁用（空串判定）
fn write_log_entry(log_dir: &str, channel: &str, message: &str, opts: &LogOptions) {
    if log_dir.is_empty() {
        return;
    }
    let now = chrono::Local::now();
    // stderr 单独写 yyyy-MM-dd.err.log，其余写主日志 yyyy-MM-dd.log
    let file_name = if opts.split_out_err && channel == "err" {
        format!("{}.err.log", now.format("%Y-%m-%d"))
    } else {
        format!("{}.log", now.format("%Y-%m-%d"))
    };
    let log_file = Path::new(log_dir).join(file_name);
    // 子进程 out/err 已按行分隔，无需转义；其余条目（钩子命令/URL/错误等）转义控制字符，
    // 防止伪造日志条目（对应 WinSW #924 日志注入）
    let text = if channel == "out" || channel == "err" {
        message.to_string()
    } else {
        escape_invisible(message)
    };
    let entry = format!("[{}] [{}] {}\r\n", now.format("%Y-%m-%d %H:%M:%S"), channel, text);
    let _guard = LOG_WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    roll_if_needed(&log_file, opts.max_size_mb, opts.backup_count);
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .map(|mut f| {
            let _ = f.write_all(entry.as_bytes());
        });
}

/// 日志大小滚动: 超过 max_size_mb 时按 .1/.2 后缀顺延，最多保留 backup_count 份；
/// 备份名 yyyy-MM-dd.log.N 供更新程序按日期前缀清理（等价 RollIfNeeded）
pub(crate) fn roll_if_needed(log_file: &Path, max_size_mb: i64, backup_count: i32) {
    if max_size_mb <= 0 || backup_count <= 0 {
        return;
    }
    let len = std::fs::metadata(log_file).map(|m| m.len()).unwrap_or(0);
    if len < (max_size_mb as u64) * 1024 * 1024 {
        return;
    }

    let oldest = PathBuf::from(format!("{}.{}", log_file.display(), backup_count));
    let _ = std::fs::remove_file(&oldest);
    for i in (1..backup_count).rev() {
        let src = PathBuf::from(format!("{}.{}", log_file.display(), i));
        if src.exists() {
            let dst = PathBuf::from(format!("{}.{}", log_file.display(), i + 1));
            let _ = std::fs::remove_file(&dst); // File.Move(overwrite:true) 语义
            let _ = std::fs::rename(&src, &dst);
        }
    }
    let first = PathBuf::from(format!("{}.1", log_file.display()));
    let _ = std::fs::remove_file(&first);
    let _ = std::fs::rename(log_file, &first);
}

/// 转义不可见/控制字符为可见序列（\r \n \t \x..），用于错误信息与日志（对应 WinSW #462/#1337）
pub(crate) fn escape_invisible(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02X}", c as u32)),
            _ => out.push(c),
        }
    }
    out
}

// ==================== Win32 回调 / 工具 ====================

/// 标记 WM_CLOSE 是否已实际发送给目标进程的窗口
static WM_CLOSE_SENT: AtomicBool = AtomicBool::new(false);

/// 吞掉 Ctrl+C，防止宿主在向子进程控制台广播时被误杀（等价 CtrlHandler）
unsafe extern "system" fn ignore_ctrl_c(_ctrl_type: u32) -> BOOL {
    BOOL(1)
}

/// 枚举窗口回调: 向属于目标 PID 的顶层窗口发送 WM_CLOSE
unsafe extern "system" fn send_wm_close(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let target_pid = lparam.0 as u32;
    let mut win_pid: u32 = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut win_pid));
        if win_pid == target_pid {
            let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
            WM_CLOSE_SENT.store(true, Ordering::SeqCst);
        }
    }
    BOOL(1)
}

/// 终止 pid 的所有后代进程（基于 Toolhelp 快照），等价于 Process.Kill(entireProcessTree) 的子树部分
fn terminate_pid_tree(root_pid: u32) {
    for desc_pid in collect_descendants(root_pid) {
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, desc_pid) {
                let _ = TerminateProcess(h, 1);
                let _ = CloseHandle(h);
            }
        }
    }
}

/// 收集 pid 的所有后代进程 ID（BFS，基于 Toolhelp 快照）
pub(crate) fn collect_descendants(root_pid: u32) -> Vec<u32> {
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return vec![];
        };

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                pairs.push((entry.th32ProcessID, entry.th32ParentProcessID));
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);

        let mut result = Vec::new();
        let mut queue = std::collections::VecDeque::from([root_pid]);
        while let Some(pid) = queue.pop_front() {
            for &(child_pid, parent_pid) in &pairs {
                if parent_pid == pid && child_pid != pid {
                    result.push(child_pid);
                    queue.push_back(child_pid);
                }
            }
        }
        result
    }
}

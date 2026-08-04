use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}
fn default_five() -> i32 {
    5
}

/// YAML 服务配置模型 — 定义将任意可执行程序注册为 Windows 服务的所有参数；
/// serde default 仅字段缺失时生效（区分"缺失"与"显式默认值"）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServiceConfig {
    // ==================== 必填字段 ====================

    /// 服务名称 — SCM 内部标识符，不可重复
    #[serde(rename = "service_name")]
    pub service_name: String,

    /// 服务显示名称 — 在 services.msc 中显示的人类可读名称
    #[serde(rename = "service_display_name")]
    pub service_display_name: String,

    /// 服务描述 — 在服务属性对话框中显示
    #[serde(rename = "service_description")]
    pub service_description: String,

    /// 目标可执行程序的完整路径
    #[serde(rename = "service_executable_path")]
    pub service_executable_path: String,

    // ==================== 可选字段 ====================

    /// 目标程序的命令行参数
    #[serde(rename = "service_executable_args")]
    pub service_executable_args: Option<String>,

    /// 启动类型: automatic | delayed_auto | manual | disabled
    #[serde(rename = "service_start_mode")]
    pub service_start_mode: Option<String>,

    /// 依赖的服务名列表，分号分隔（如 "EventLog;WinRM"）
    #[serde(rename = "service_dependencies")]
    pub service_dependencies: Option<String>,

    /// 运行服务的 Windows 账户（如 "NT AUTHORITY\NetworkService"）
    #[serde(rename = "service_account")]
    pub service_account: Option<String>,

    /// 服务账户密码（仅自定义账户需要）
    #[serde(rename = "service_password")]
    pub service_password: Option<String>,

    /// 注入目标进程的环境变量
    #[serde(default)]
    pub env: Option<std::collections::HashMap<String, String>>,

    /// 失败计数重置周期（秒），默认 86400（24 小时）
    #[serde(rename = "failure_reset_sec", default)]
    pub failure_reset_sec: i32,

    /// 崩溃后自动重启延迟（毫秒），默认 60000（60 秒）
    #[serde(rename = "restart_delay_ms", default)]
    pub restart_delay_ms: i32,

    /// 停止时是否强制终止整棵进程树，默认 true；设为 false 时仅终止主进程（对应 WinSW #990）
    #[serde(rename = "kill_process_tree", default = "default_true")]
    pub kill_process_tree: bool,

    /// 原地注册模式（默认 false）: 不复制宿主到 ProgramData，直接用当前 silanes64.exe 注册
    /// （yaml 须与 exe 同名同目录），此类服务不纳入平台开机更新/清理
    #[serde(rename = "deploy_inplace", default)]
    pub deploy_inplace: bool,

    // ==================== 生命周期钩子（可选） ====================

    /// 启动前钩子命令 — 在拉起目标进程前执行（cmd.exe 语义，失败不阻断）
    #[serde(rename = "prestart_command")]
    pub prestart_command: Option<String>,

    /// 停止后钩子命令 — 在目标进程停止后执行（cmd.exe 语义，失败不阻断）
    #[serde(rename = "poststop_command")]
    pub poststop_command: Option<String>,

    // ==================== 启动前下载（可选） ====================

    /// 启动前下载 URL — 配置后宿主在启动前确保该可执行文件已就位
    #[serde(rename = "download_url")]
    pub download_url: Option<String>,

    /// 下载目标路径 — 相对路径基于服务部署目录；省略时取 service_executable_path 的文件名
    #[serde(rename = "download_to")]
    pub download_to: Option<String>,

    /// 下载文件 SHA-256 校验值（小写十六进制）— 缺失或匹配失败时重新下载
    #[serde(rename = "download_sha256")]
    pub download_sha256: Option<String>,

    /// 下载失败是否导致服务启动失败，默认 true
    #[serde(rename = "download_fail_on_error", default = "default_true")]
    pub download_fail_on_error: bool,

    // ==================== 日志（可选） ====================

    /// 是否写入服务日志，默认 true；设为 false 可彻底关闭宿主日志（含钩子/下载输出）
    #[serde(rename = "log_enabled", default = "default_true")]
    pub log_enabled: bool,

    /// 日志目录 — 相对路径基于服务部署目录；省略时默认 logs 子目录
    #[serde(rename = "log_dir")]
    pub log_dir: Option<String>,

    /// 单日日志大小上限（MB），超过后滚动备份；0 表示不限（默认）
    #[serde(rename = "log_max_size_mb", default)]
    pub log_max_size_mb: i64,

    /// 大小滚动保留的备份份数，默认 5
    #[serde(rename = "log_max_backup_count", default = "default_five")]
    pub log_max_backup_count: i32,

    /// 是否把子进程 stderr 单独写入 yyyy-MM-dd.err.log，默认 false（合并写入主日志）
    #[serde(rename = "log_split_out_err", default)]
    pub log_split_out_err: bool,
}

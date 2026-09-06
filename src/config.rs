use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 顶层配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub repo: RepoConfig,
    pub notify: NotifyConfig,
    #[serde(default)]
    pub retention: RetentionConfig,
    pub tasks: Vec<TaskConfig>,
    /// rustic.exe 路径，默认同目录下 bin/rustic.exe
    #[serde(default)]
    pub rustic_path: Option<String>,
    /// 日志目录，默认同目录下 logs
    #[serde(default)]
    pub log_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    /// 仓库路径
    pub path: String,
    /// 仓库密码
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyConfig {
    /// 企业微信 webhook 完整 URL
    pub webhook: String,
    /// 是否启用微盘上传（大文件走微盘）
    #[serde(default = "default_true")]
    pub wedrive_enabled: bool,
    /// webhook 文件大小上限（字节），默认 20MB
    #[serde(default = "default_max_webhook_size")]
    pub max_webhook_size: u64,
    /// 大文件自动分片发送（超过 max_webhook_size 时拆成多个包）
    #[serde(default = "default_true")]
    pub split_large_files: bool,
    /// 分片单包大小上限（字节），默认 18MB（留余量）
    #[serde(default = "default_split_size")]
    pub split_part_size: u64,
}

fn default_true() -> bool {
    true
}
fn default_max_webhook_size() -> u64 {
    20 * 1024 * 1024
}
fn default_split_size() -> u64 {
    15 * 1024 * 1024
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionConfig {
    #[serde(default = "default_daily")]
    pub daily: u32,
    #[serde(default = "default_weekly")]
    pub weekly: u32,
    #[serde(default = "default_monthly")]
    pub monthly: u32,
    #[serde(default = "default_yearly")]
    pub yearly: u32,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            daily: 15,
            weekly: 12,
            monthly: 12,
            yearly: 3,
        }
    }
}

fn default_daily() -> u32 { 15 }
fn default_weekly() -> u32 { 12 }
fn default_monthly() -> u32 { 12 }
fn default_yearly() -> u32 { 3 }

/// 备份任务计划类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleType {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl ScheduleType {
    pub fn as_tag(&self) -> &'static str {
        match self {
            ScheduleType::Daily => "daily",
            ScheduleType::Weekly => "weekly",
            ScheduleType::Monthly => "monthly",
            ScheduleType::Yearly => "yearly",
        }
    }
}

/// 单个备份任务配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    /// 任务名称（显示用，也作为 tag）
    pub name: String,
    /// 源目录路径
    pub source: String,
    /// 计划类型
    #[serde(default = "default_schedule")]
    pub schedule: ScheduleType,
    /// 执行时间 HH:MM
    #[serde(default = "default_time")]
    pub time: String,
    /// 每周几执行（仅 schedule=weekly 时生效），0=周日
    #[serde(default)]
    pub weekday: Option<u32>,
    /// 每月几号执行（仅 schedule=monthly 时生效）
    #[serde(default)]
    pub monthday: Option<u32>,
    /// glob 包含模式，不传则备份全部
    #[serde(default)]
    pub include: Vec<String>,
    /// glob 排除模式
    #[serde(default)]
    pub exclude: Vec<String>,
    /// 正则排除模式（应用层过滤）
    #[serde(default)]
    pub exclude_regex: Vec<String>,
    /// 是否打包根目录文件发送到企业微信
    #[serde(default = "default_true")]
    pub zip_root_files: bool,
    /// 根目录文件匹配模式（默认 *.xlsx, *.xls）
    #[serde(default = "default_root_patterns")]
    pub root_file_patterns: Vec<String>,
    /// 通知相关开关
    #[serde(default)]
    pub notify: TaskNotifyConfig,
    /// 是否启用该任务
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_schedule() -> ScheduleType { ScheduleType::Daily }
fn default_time() -> String { "07:30".to_string() }
fn default_root_patterns() -> Vec<String> {
    vec!["*.xlsx".to_string(), "*.xls".to_string()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNotifyConfig {
    /// 第一次备份（无历史快照）时跳过文件发送
    #[serde(default = "default_true")]
    pub skip_first_run: bool,
    /// 增量模式：只打包变化的文件发送
    #[serde(default = "default_true")]
    pub incremental: bool,
    /// 递归扫描子文件夹打包发送（默认 false，只发根目录文件）
    #[serde(default = "default_false")]
    pub recursive_send: bool,
}

fn default_false() -> bool { false }

impl Default for TaskNotifyConfig {
    fn default() -> Self {
        Self {
            skip_first_run: true,
            incremental: true,
            recursive_send: false,
        }
    }
}

impl Config {
    /// 从文件加载配置（自动把相对路径解析为相对于配置文件所在目录的绝对路径）
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
        let mut config: Config = serde_json::from_str(&content)
            .with_context(|| format!("解析配置文件失败: {}", path.display()))?;

        // 配置文件所在目录，用于解析相对路径
        let config_dir = path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        config.resolve_paths(&config_dir);
        Ok(config)
    }

    /// 把所有相对路径解析为相对于 base_dir 的绝对路径
    fn resolve_paths(&mut self, base_dir: &Path) {
        // 仓库路径
        self.repo.path = Self::resolve_abs(&self.repo.path, base_dir);

        // 日志目录
        if let Some(d) = &self.log_dir {
            self.log_dir = Some(Self::resolve_abs(d, base_dir));
        }

        // rustic.exe 路径
        if let Some(p) = &self.rustic_path {
            self.rustic_path = Some(Self::resolve_abs(p, base_dir));
        }

        // 任务源路径
        for task in &mut self.tasks {
            task.source = Self::resolve_abs(&task.source, base_dir);
        }
    }

    /// 把相对路径转为绝对路径（已经是绝对路径则不变）
    fn resolve_abs(path: &str, base_dir: &Path) -> String {
        let p = PathBuf::from(path);
        if p.is_absolute() {
            path.to_string()
        } else {
            base_dir.join(p).to_string_lossy().to_string()
        }
    }

    /// 保存到文件
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// 获取 rustic.exe 路径
    pub fn rustic_exe(&self) -> PathBuf {
        if let Some(p) = &self.rustic_path {
            PathBuf::from(p)
        } else {
            // 默认同目录下 bin/rustic.exe
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."));
            exe_dir.join("bin").join("rustic.exe")
        }
    }

    /// 获取日志目录
    pub fn log_dir(&self) -> PathBuf {
        if let Some(d) = &self.log_dir {
            PathBuf::from(d)
        } else {
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."));
            exe_dir.join("logs")
        }
    }

    /// 获取临时目录
    pub fn temp_dir(&self) -> PathBuf {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        exe_dir.join("temp")
    }

    /// 按序号获取任务（1-based）
    pub fn task_by_index(&self, idx: usize) -> Option<&TaskConfig> {
        if idx >= 1 && idx <= self.tasks.len() {
            Some(&self.tasks[idx - 1])
        } else {
            None
        }
    }
}

/// 生成默认配置示例
pub fn default_config_example() -> Config {
    Config {
        repo: RepoConfig {
            path: "./repo".to_string(),
            password: "请修改为你的仓库密码".to_string(),
        },
        notify: NotifyConfig {
            webhook: "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=你的key".to_string(),
            wedrive_enabled: false,
            max_webhook_size: 20 * 1024 * 1024,
            split_large_files: true,
            split_part_size: 18 * 1024 * 1024,
        },
        retention: RetentionConfig::default(),
        rustic_path: None,
        log_dir: None,
        tasks: vec![
            TaskConfig {
                name: "检测数据".to_string(),
                source: r"\\nas\质量部\检测数据".to_string(),
                schedule: ScheduleType::Daily,
                time: "07:30".to_string(),
                weekday: None,
                monthday: None,
                include: vec![],
                exclude: vec!["~$*".to_string(), "*.tmp".to_string()],
                exclude_regex: vec![],
                zip_root_files: true,
                root_file_patterns: default_root_patterns(),
                notify: TaskNotifyConfig::default(),
                enabled: true,
            },
        ],
    }
}

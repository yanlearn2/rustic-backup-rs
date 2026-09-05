mod config;
mod download;
mod logger;
mod notify;
mod plan;
mod rustic;
mod scheduler;
mod setup;
mod task;
mod wedrive;
mod zip_util;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

use config::Config;
use logger::Logger;
use plan::TaskScheduler;
use rustic::{format_size, retention_tier, Rustic};
use scheduler::Scheduler;
use task::TaskManager;

const TASK_NAME: &str = "RusticBackupService";
const DEFAULT_CONFIG: &str = "config.json";

#[derive(Parser)]
#[command(name = "rustic-backup", version, about = "Rustic 备份管理工具")]
struct Cli {
    /// 配置文件路径
    #[arg(short, long, default_value = DEFAULT_CONFIG)]
    config: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 初始化备份仓库
    Init,
    /// 注册 Windows 计划任务
    Install {
        /// 执行时间 HH:MM
        #[arg(long, default_value = "07:30")]
        time: String,
    },
    /// 移除计划任务
    Uninstall,
    /// 手动触发备份
    Run {
        /// 只执行指定任务（序号，1-based）
        #[arg(long)]
        task: Option<usize>,
        /// 跳过企业微信通知（只备份不发送）
        #[arg(long, conflicts_with = "send")]
        no_notify: bool,
        /// 强制发送（忽略 skip_first_run，首次也发）
        #[arg(long)]
        send: bool,
    },
    /// 查看所有备份任务状态
    Tasks {
        /// 启用任务
        #[arg(long)]
        enable: Option<usize>,
        /// 禁用任务
        #[arg(long)]
        disable: Option<usize>,
    },
    /// 查看备份状态（最近结果 + 仓库信息）
    Status,
    /// 列出快照
    List {
        /// 只看指定任务的快照
        #[arg(long)]
        task: Option<usize>,
        /// 只看最近 N 天
        #[arg(long)]
        last: Option<u32>,
        /// 按标签过滤
        #[arg(long)]
        tag: Option<String>,
        /// 显示完整快照ID
        #[arg(long)]
        long: bool,
    },
    /// 恢复文件
    Restore {
        /// 快照ID
        snapshot_id: String,
        /// 恢复目标目录
        #[arg(long)]
        target: String,
        /// 只恢复指定文件/路径
        #[arg(long)]
        include: Option<String>,
    },
    /// 查看日志
    Logs {
        /// 指定日期 YYYYMMDD
        #[arg(long)]
        date: Option<String>,
    },
    /// 清理旧快照 + 空间回收
    Prune {
        /// 按任务标签清理
        #[arg(long)]
        task: Option<usize>,
        /// 只预览，不实际执行
        #[arg(long)]
        dry_run: bool,
        /// 确认执行（不加则默认 dry-run）
        #[arg(long)]
        yes: bool,
    },
    /// 生成默认配置文件
    GenConfig,
    /// 交互式配置引导
    Setup,
    /// 下载/更新 rustic.exe
    Download,
}

fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(cli) {
        eprintln!("错误: {}", e);
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let config_path = PathBuf::from(&cli.config);

    match &cli.command {
        Commands::GenConfig => {
            return cmd_gen_config(&config_path);
        }
        Commands::Setup => {
            return setup::run_setup(&config_path);
        }
        Commands::Download => {
            return cmd_download();
        }
        Commands::Init => {
            let config = load_config(&config_path)?;
            download::ensure_rustic(config.rustic_path.as_deref())?;
            return cmd_init(&config);
        }
        Commands::Install { time } => {
            let config = load_config(&config_path)?;
            return cmd_install(&config, time);
        }
        Commands::Uninstall => {
            return cmd_uninstall();
        }
        Commands::Run { task, no_notify, send } => {
            let config = load_config(&config_path)?;
            download::ensure_rustic(config.rustic_path.as_deref())?;
            let logger = Logger::new(config.log_dir());
            return cmd_run(&config, &logger, *task, *no_notify, *send);
        }
        Commands::Tasks { enable, disable } => {
            return cmd_tasks(&config_path, *enable, *disable);
        }
        Commands::Status => {
            let config = load_config(&config_path)?;
            download::ensure_rustic(config.rustic_path.as_deref())?;
            return cmd_status(&config);
        }
        Commands::List { task, last, tag, long } => {
            let config = load_config(&config_path)?;
            download::ensure_rustic(config.rustic_path.as_deref())?;
            return cmd_list(&config, *task, *last, tag.as_deref(), *long);
        }
        Commands::Restore { snapshot_id, target, include } => {
            let config = load_config(&config_path)?;
            download::ensure_rustic(config.rustic_path.as_deref())?;
            return cmd_restore(&config, snapshot_id, target, include.as_deref());
        }
        Commands::Logs { date } => {
            let config = load_config(&config_path)?;
            let logger = Logger::new(config.log_dir());
            return cmd_logs(&logger, date.as_deref());
        }
        Commands::Prune { task, dry_run, yes } => {
            let config = load_config(&config_path)?;
            download::ensure_rustic(config.rustic_path.as_deref())?;
            let logger = Logger::new(config.log_dir());
            return cmd_prune(&config, &logger, *task, *dry_run, *yes);
        }
    }
}

fn load_config(path: &Path) -> Result<Config> {
    if !path.exists() {
        anyhow::bail!(
            "配置文件不存在: {}\n请先运行 'rustic-backup gen-config' 生成默认配置",
            path.display()
        );
    }
    Config::load(path)
}

fn cmd_gen_config(path: &Path) -> Result<()> {    if path.exists() {
        anyhow::bail!("配置文件已存在: {}（如需重新生成请先删除）", path.display());
    }
    let config = config::default_config_example();
    config.save(path)?;
    println!("已生成默认配置文件: {}", path.display());
    println!("请编辑配置文件，填写仓库密码、源路径和 webhook 后使用");
    Ok(())
}

fn cmd_download() -> Result<()> {
    let bin_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bin");
    let existing = bin_dir.join("rustic.exe");
    if existing.exists() {
        println!("检测到已有 rustic.exe，将重新下载最新版");
        let _ = std::fs::remove_file(&existing);
    }
    let path = download::ensure_rustic(None)?;
    println!();
    println!("✅ rustic.exe 已就绪: {}", path.display());
    Ok(())
}

fn cmd_init(config: &Config) -> Result<()> {
    let rustic = Rustic::new(config);
    println!("初始化仓库: {}", config.repo.path);
    rustic.init()?;
    println!("✅ 仓库初始化成功");
    println!("⚠️  请妥善保管仓库密码: {}", config.repo.password);
    Ok(())
}

fn cmd_install(_config: &Config, time: &str) -> Result<()> {
    let exe_path = std::env::current_exe()?
        .to_string_lossy()
        .to_string();
    let scheduler = TaskScheduler::new(TASK_NAME);

    if scheduler.is_installed()? {
        println!("计划任务已存在，先移除旧任务...");
        scheduler.uninstall()?;
    }

    scheduler.install(&exe_path, time)?;
    println!("✅ 计划任务注册成功");
    println!("   任务名: {}", TASK_NAME);
    println!("   执行时间: 每天 {}", time);
    println!("   程序: {}", exe_path);
    Ok(())
}

fn cmd_uninstall() -> Result<()> {
    let scheduler = TaskScheduler::new(TASK_NAME);
    scheduler.uninstall()?;
    println!("✅ 计划任务已移除");
    println!("   仓库和配置文件保留，如需彻底删除请手动删除");
    Ok(())
}

fn cmd_run(config: &Config, logger: &Logger, task_idx: Option<usize>, no_notify: bool, send: bool) -> Result<()> {
    let scheduler = Scheduler::new(config, logger);
    let notify_enabled = !no_notify;
    let force_notify = send;

    if let Some(idx) = task_idx {
        println!("手动执行任务 #{}", idx);
        let result = scheduler.run_task_by_index(idx, notify_enabled, force_notify)?;
        println!("任务: {} | 成功: {} | 快照: {:?}", result.task_name, result.success, result.snapshot_id);
    } else {
        println!("执行所有到期任务...");
        let results = scheduler.run_all_due(notify_enabled, force_notify)?;
        println!("\n执行完成，共 {} 个任务", results.len());
        for r in &results {
            println!("  [{}] {} | 快照: {:?}", if r.success { "✅" } else { "❌" }, r.task_name, r.snapshot_id);
        }
    }
    Ok(())
}

fn cmd_tasks(config_path: &Path, enable: Option<usize>, disable: Option<usize>) -> Result<()> {
    if let Some(idx) = enable {
        TaskManager::enable(config_path, idx)?;
        println!("✅ 任务 #{} 已启用", idx);
        return Ok(());
    }
    if let Some(idx) = disable {
        TaskManager::disable(config_path, idx)?;
        println!("✅ 任务 #{} 已禁用", idx);
        return Ok(());
    }

    let config = load_config(config_path)?;
    let tasks = TaskManager::list(&config);

    println!("备份任务（config.json 中定义）");
    println!("{}", "─".repeat(70));
    for t in &tasks {
        let status = if t.enabled { "启用" } else { "禁用" };
        println!("[{}] {}", t.index, t.name);
        println!("    源路径: {}", t.source);
        println!("    计划:   {} {}", t.schedule, t.time);
        if !t.include.is_empty() {
            println!("    包含:   {}", t.include.join(", "));
        }
        if !t.exclude.is_empty() {
            println!("    排除:   {}", t.exclude.join(", "));
        }
        println!("    打包发送: {} | 首次跳过: {} | 增量: {}", t.zip_root_files, t.skip_first_run, t.incremental);
        println!("    状态:   {}", status);
        println!();
    }

    // Windows 计划任务状态
    let scheduler = TaskScheduler::new(TASK_NAME);
    println!("Windows 计划任务");
    println!("{}", "─".repeat(70));
    match scheduler.status() {
        Ok(s) => {
            println!("任务名:   {}", s.name);
            println!("状态:     {}", s.status);
            println!("下次运行: {}", s.next_run);
            println!("上次运行: {}", s.last_run);
            println!("上次结果: {}", s.last_result);
        }
        Err(_) => {
            println!("未注册（运行 'install' 注册）");
        }
    }
    Ok(())
}

fn cmd_status(config: &Config) -> Result<()> {
    let rustic = Rustic::new(config);

    println!("备份状态");
    println!("{}", "─".repeat(70));

    // 仓库信息
    match rustic.repoinfo() {
        Ok(stats) => {
            println!("仓库路径: {}", config.repo.path);
            println!("仓库大小: {}", format_size(stats.total_size));
            println!("快照数量: {}", stats.snapshot_count);
        }
        Err(e) => {
            println!("仓库信息获取失败: {}", e);
        }
    }
    println!();

    // 各任务最近快照
    println!("各任务最近备份:");
    for (i, task) in config.tasks.iter().enumerate() {
        match rustic.latest_snapshot(task) {
            Ok(Some(snap)) => {
                let files = snap.summary.as_ref().and_then(|s| s.files_total).unwrap_or(0);
                println!("  [{}] {} | {} | {} 文件 | {}",
                    i + 1, task.name, snap.time, files,
                    if snap.id.len() >= 8 { &snap.id[..8] } else { &snap.id });
            }
            Ok(None) => {
                println!("  [{}] {} | 暂无备份", i + 1, task.name);
            }
            Err(e) => {
                println!("  [{}] {} | 查询失败: {}", i + 1, task.name, e);
            }
        }
    }

    // 计划任务状态
    println!();
    let scheduler = TaskScheduler::new(TASK_NAME);
    match scheduler.status() {
        Ok(s) => println!("计划任务: {} | 下次运行: {}", s.status, s.next_run),
        Err(_) => println!("计划任务: 未注册"),
    }
    Ok(())
}

fn cmd_list(config: &Config, task_idx: Option<usize>, last_days: Option<u32>, tag: Option<&str>, long_id: bool) -> Result<()> {
    let rustic = Rustic::new(config);

    // 构建过滤标签
    let mut tags: Vec<&str> = Vec::new();
    if let Some(t) = tag {
        tags.push(t);
    }
    if let Some(idx) = task_idx {
        if let Some(task) = config.task_by_index(idx) {
            tags.push(task.schedule.as_tag());
            tags.push(&task.name);
        }
    }

    let mut snapshots = rustic.snapshots(&tags)?;

    // 按时间倒序
    snapshots.sort_by(|a, b| b.time.cmp(&a.time));

    // 最近 N 天过滤
    if let Some(days) = last_days {
        let cutoff = chrono::Local::now() - chrono::Duration::days(days as i64);
        snapshots.retain(|s| {
            chrono::DateTime::parse_from_rfc3339(&s.time)
                .map(|dt| dt.with_timezone(&chrono::Local) > cutoff)
                .unwrap_or(true)
        });
    }

    let repo_stats = rustic.repoinfo().unwrap_or_default();

    println!("快照列表（共 {} 个，仓库 {})", snapshots.len(), format_size(repo_stats.total_size));
    println!("{}", "─".repeat(90));
    println!("{:<10} {:<20} {:<16} {:>8} {:>10} {:<6}",
        "ID", "时间", "任务", "文件数", "大小", "保留");
    println!("{}", "─".repeat(90));

    for snap in &snapshots {
        let id_display = if long_id { snap.id.clone() } else if snap.id.len() >= 8 { snap.id[..8].to_string() } else { snap.id.clone() };
        let task_name = snap.tags.iter()
            .find(|t| !["daily", "weekly", "monthly", "yearly"].contains(&t.as_str()))
            .cloned()
            .unwrap_or_else(|| "-".to_string());
        let files = snap.summary.as_ref().and_then(|s| s.files_total).unwrap_or(0);
        let size = snap.summary.as_ref().and_then(|s| s.bytes_processed).unwrap_or(0);
        let tier = retention_tier(snap);

        println!("{:<10} {:<20} {:<16} {:>8} {:>10} {:<6}",
            id_display, snap.time, task_name, files, format_size(size), tier);
    }
    Ok(())
}

fn cmd_restore(config: &Config, snapshot_id: &str, target: &str, include: Option<&str>) -> Result<()> {
    let rustic = Rustic::new(config);
    let target_path = PathBuf::from(target);

    println!("恢复快照 {} → {}", snapshot_id, target);
    if let Some(inc) = include {
        println!("只恢复: {}", inc);
    }

    rustic.restore(snapshot_id, &target_path, include)?;
    println!("✅ 恢复完成");
    Ok(())
}

fn cmd_logs(logger: &Logger, date: Option<&str>) -> Result<()> {
    let content = match date {
        Some(d) => logger.read_date(d),
        None => logger.read_today(),
    };
    println!("{}", content);
    Ok(())
}

fn cmd_prune(config: &Config, logger: &Logger, task_idx: Option<usize>, dry_run: bool, yes: bool) -> Result<()> {
    let rustic = Rustic::new(config);

    // 先校验仓库
    println!("校验仓库完整性...");
    if let Err(e) = rustic.check() {
        anyhow::bail!("仓库校验失败，拒绝执行 prune: {}", e);
    }
    println!("✅ 仓库校验通过");

    let tag = task_idx.and_then(|idx| config.task_by_index(idx).map(|t| t.name.clone()));
    let actually_run = yes && !dry_run;

    if !actually_run {
        println!("\n[预览模式] 以下操作不会实际执行");
    }

    let output = rustic.forget_and_prune(&config.retention, tag.as_deref(), !actually_run)?;
    println!("{}", output);

    if actually_run {
        // 清理后再次校验
        println!("清理后校验仓库...");
        if let Err(e) = rustic.check() {
            logger.error(&format!("prune 后校验失败: {}", e));
            anyhow::bail!("prune 后仓库校验失败: {}", e);
        }
        println!("✅ 清理完成，仓库校验通过");

        // 发送通知
        let notifier = notify::WechatNotifier::new(&config.notify.webhook);
        let _ = notifier.send_text(&format!(
            "🧹 备份仓库清理完成\n保留策略: 日{} 周{} 月{} 年{}\n仓库: {}",
            config.retention.daily, config.retention.weekly,
            config.retention.monthly, config.retention.yearly,
            config.repo.path
        ));
    } else {
        println!("\n加 --yes 参数确认执行实际清理");
    }

    Ok(())
}

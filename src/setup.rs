use anyhow::Result;
use inquire::{Confirm, Password, Select, Text};
use std::path::Path;

use crate::config::{
    Config, NotifyConfig, RepoConfig, RetentionConfig, ScheduleType,
    TaskConfig, TaskNotifyConfig,
};

/// 入口：判断全新创建 or 编辑模式
pub fn run_setup(output_path: &Path) -> Result<()> {
    println!("╔══════════════════════════════════════╗");
    println!("║   rustic-backup 配置向导              ║");
    println!("╚══════════════════════════════════════╝");
    println!();

    if output_path.exists() {
        let config = Config::load(output_path)?;
        edit_mode(output_path, config)
    } else {
        let config = create_new_config()?;
        config.save(output_path)?;
        print_next_steps(output_path, false);
        Ok(())
    }
}

// ═══════════════════════════════════════
// 编辑模式
// ═══════════════════════════════════════

fn edit_mode(output_path: &Path, mut config: Config) -> Result<()> {
    println!("检测到现有配置，进入编辑模式。");
    println!("每项显示当前值，回车保留，输入新值修改。");
    println!();

    loop {
        let options = vec![
            "1. 仓库配置（路径、密码）",
            "2. 通知配置（webhook、微盘）",
            "3. 保留策略（日/周/月/年）",
            "4. 备份任务管理",
            "5. 重新开始（清空现有配置，从头创建）",
            "6. 保存并退出",
            "7. 放弃修改",
        ];
        let choice = Select::new("选择要修改的配置项:", options)
            .with_starting_cursor(0)
            .prompt()?;

        match choice {
            s if s.starts_with("1.") => edit_repo(&mut config)?,
            s if s.starts_with("2.") => edit_notify(&mut config)?,
            s if s.starts_with("3.") => edit_retention(&mut config)?,
            s if s.starts_with("4.") => manage_tasks(&mut config)?,
            s if s.starts_with("5.") => {
                let confirm = Confirm::new("确认清空现有配置，从头开始创建？")
                    .with_default(false)
                    .prompt()?;
                if confirm {
                    println!();
                    config = create_new_config()?;
                    println!();
                    println!("✅ 已重新创建配置，可继续编辑或保存退出。");
                }
            }
            s if s.starts_with("6.") => {
                config.save(output_path)?;
                println!();
                println!("✅ 配置已保存到: {}", output_path.display());
                break;
            }
            s if s.starts_with("7.") => {
                let confirm = Confirm::new("确认放弃所有修改？")
                    .with_default(false)
                    .prompt()?;
                if confirm {
                    println!("已放弃修改。");
                    break;
                }
            }
            _ => {}
        }
        println!();
    }
    Ok(())
}

fn edit_repo(config: &mut Config) -> Result<()> {
    println!("── 仓库配置（当前值显示在括号内）──");
    config.repo.path = Text::new("仓库路径:")
        .with_default(&config.repo.path)
        .prompt()?;

    let change_pwd = Confirm::new("修改仓库密码？")
        .with_default(false)
        .prompt()?;
    if change_pwd {
        let pwd = Password::new("新密码:")
            .with_display_mode(inquire::PasswordDisplayMode::Masked)
            .without_confirmation()
            .prompt()?;
        let pwd2 = Password::new("确认新密码:")
            .with_display_mode(inquire::PasswordDisplayMode::Masked)
            .without_confirmation()
            .prompt()?;
        if pwd != pwd2 {
            anyhow::bail!("两次密码不一致，未修改");
        }
        config.repo.password = pwd;
        println!("密码已更新。");
    }
    Ok(())
}

fn edit_notify(config: &mut Config) -> Result<()> {
    println!("── 通知配置 ──");
    config.notify.webhook = Text::new("企业微信 Webhook URL:")
        .with_default(&config.notify.webhook)
        .prompt()?;
    config.notify.wedrive_enabled = Confirm::new("启用微盘上传?")
        .with_default(config.notify.wedrive_enabled)
        .prompt()?;
    config.notify.split_large_files = Confirm::new("大文件自动分片发送?")
        .with_default(config.notify.split_large_files)
        .prompt()?;
    Ok(())
}

fn edit_retention(config: &mut Config) -> Result<()> {
    println!("── 保留策略 ──");
    config.retention.daily = Text::new("每日保留天数:")
        .with_default(&config.retention.daily.to_string())
        .prompt()?
        .parse()
        .unwrap_or(config.retention.daily);
    config.retention.weekly = Text::new("每周保留周数:")
        .with_default(&config.retention.weekly.to_string())
        .prompt()?
        .parse()
        .unwrap_or(config.retention.weekly);
    config.retention.monthly = Text::new("每月保留月数:")
        .with_default(&config.retention.monthly.to_string())
        .prompt()?
        .parse()
        .unwrap_or(config.retention.monthly);
    config.retention.yearly = Text::new("每年保留年数:")
        .with_default(&config.retention.yearly.to_string())
        .prompt()?
        .parse()
        .unwrap_or(config.retention.yearly);
    Ok(())
}

fn manage_tasks(config: &mut Config) -> Result<()> {
    loop {
        println!();
        println!("── 备份任务列表 ──");
        for (i, t) in config.tasks.iter().enumerate() {
            let status = if t.enabled { "启用" } else { "禁用" };
            println!("  [{}] {} | {} | {} | {}",
                i + 1, t.name, t.schedule.as_tag(), t.time, status);
        }
        if config.tasks.is_empty() {
            println!("  （暂无任务）");
        }
        println!();

        let options = vec![
            "添加新任务",
            "编辑任务",
            "删除任务",
            "启用/禁用任务",
            "返回主菜单",
        ];
        let choice = Select::new("任务操作:", options)
            .with_starting_cursor(0)
            .prompt()?;

        match choice {
            "添加新任务" => {
                let task = prompt_task(None)?;
                config.tasks.push(task);
                println!("✅ 任务已添加");
            }
            "编辑任务" => {
                if config.tasks.is_empty() {
                    println!("暂无任务可编辑");
                    continue;
                }
                let idx = select_task_index(&config.tasks, "选择要编辑的任务:")?;
                let existing = config.tasks[idx].clone();
                let updated = prompt_task(Some(&existing))?;
                config.tasks[idx] = updated;
                println!("✅ 任务已更新");
            }
            "删除任务" => {
                if config.tasks.is_empty() {
                    println!("暂无任务可删除");
                    continue;
                }
                let idx = select_task_index(&config.tasks, "选择要删除的任务:")?;
                let name = config.tasks[idx].name.clone();
                let confirm = Confirm::new(&format!("确认删除任务「{}」？", name))
                    .with_default(false)
                    .prompt()?;
                if confirm {
                    config.tasks.remove(idx);
                    println!("✅ 任务已删除");
                }
            }
            "启用/禁用任务" => {
                if config.tasks.is_empty() {
                    println!("暂无任务");
                    continue;
                }
                let idx = select_task_index(&config.tasks, "选择任务:")?;
                config.tasks[idx].enabled = !config.tasks[idx].enabled;
                let status = if config.tasks[idx].enabled { "启用" } else { "禁用" };
                println!("✅ 任务已{}", status);
            }
            "返回主菜单" => break,
            _ => {}
        }
    }
    Ok(())
}

fn select_task_index(tasks: &[TaskConfig], prompt: &str) -> Result<usize> {
    let options: Vec<String> = tasks.iter()
        .enumerate()
        .map(|(i, t)| format!("[{}] {}", i + 1, t.name))
        .collect();
    let sel = Select::new(prompt, options)
        .with_starting_cursor(0)
        .prompt()?;
    // 解析序号
    let idx: usize = sel[1..].split(']').next().unwrap_or("0").trim().parse().unwrap_or(1);
    Ok(idx - 1)
}

// ═══════════════════════════════════════
// 全新创建模式
// ═══════════════════════════════════════

fn create_new_config() -> Result<Config> {
    // 仓库
    println!("── 仓库配置 ──");
    let repo_path = Text::new("仓库路径:")
        .with_default("./repo")
        .prompt()?;
    let password = prompt_password()?;
    println!();

    // 通知
    println!("── 通知配置 ──");
    let webhook = Text::new("企业微信 Webhook URL:")
        .with_placeholder("https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=xxx")
        .prompt()?;
    let wedrive_enabled = Confirm::new("启用微盘上传（大文件走微盘）?")
        .with_default(false)
        .prompt()?;
    let split_large_files = Confirm::new("大文件自动分片发送（超20MB拆成多个包）?")
        .with_default(true)
        .prompt()?;
    println!();

    // 保留策略
    println!("── 保留策略 ──");
    let retention = prompt_retention()?;
    println!();

    // 任务
    let mut tasks = Vec::new();
    loop {
        println!("── 备份任务 #{} ──", tasks.len() + 1);
        let task = prompt_task(None)?;
        tasks.push(task);
        println!();
        let add_more = Confirm::new(&format!("继续添加任务？（当前 {} 个）", tasks.len()))
            .with_default(false)
            .prompt()?;
        if !add_more {
            break;
        }
        println!();
    }

    Ok(Config {
        repo: RepoConfig { path: repo_path, password },
        notify: NotifyConfig {
            webhook,
            wedrive_enabled,
            max_webhook_size: 20 * 1024 * 1024,
            split_large_files,
            split_part_size: 18 * 1024 * 1024,
        },
        retention,
        tasks,
        rustic_path: None,
        log_dir: None,
    })
}

// ═══════════════════════════════════════
// 通用提示函数
// ═══════════════════════════════════════

fn prompt_password() -> Result<String> {
    let password = Password::new("仓库密码:")
        .with_display_mode(inquire::PasswordDisplayMode::Masked)
        .without_confirmation()
        .prompt()?;
    let password_confirm = Password::new("确认密码:")
        .with_display_mode(inquire::PasswordDisplayMode::Masked)
        .without_confirmation()
        .prompt()?;
    if password != password_confirm {
        anyhow::bail!("两次密码不一致");
    }
    Ok(password)
}

fn prompt_retention() -> Result<RetentionConfig> {
    let daily: u32 = Text::new("每日保留天数:")
        .with_default("15")
        .prompt()?
        .parse().unwrap_or(15);
    let weekly: u32 = Text::new("每周保留周数:")
        .with_default("12")
        .prompt()?
        .parse().unwrap_or(12);
    let monthly: u32 = Text::new("每月保留月数:")
        .with_default("12")
        .prompt()?
        .parse().unwrap_or(12);
    let yearly: u32 = Text::new("每年保留年数:")
        .with_default("3")
        .prompt()?
        .parse().unwrap_or(3);
    Ok(RetentionConfig { daily, weekly, monthly, yearly })
}

/// 提示输入一个任务，existing 为 Some 时进入编辑模式（现有值作为默认）
fn prompt_task(existing: Option<&TaskConfig>) -> Result<TaskConfig> {
    let name = Text::new("任务名称:")
        .with_placeholder("例如：检测数据")
        .with_default(existing.map(|t| t.name.as_str()).unwrap_or(""))
        .prompt()?;

    let source = Text::new("源路径:")
        .with_placeholder(r"例如：\\nas\质量部\检测数据")
        .with_default(existing.map(|t| t.source.as_str()).unwrap_or(""))
        .prompt()?;

    let schedule_options = vec!["daily（每天）", "weekly（每周）", "monthly（每月）", "yearly（每年）"];
    let current_schedule_idx = match existing.map(|t| t.schedule) {
        Some(ScheduleType::Daily) => 0,
        Some(ScheduleType::Weekly) => 1,
        Some(ScheduleType::Monthly) => 2,
        Some(ScheduleType::Yearly) => 3,
        None => 0,
    };
    let schedule_sel = Select::new("计划类型:", schedule_options)
        .with_starting_cursor(current_schedule_idx)
        .prompt()?;
    let schedule = match schedule_sel {
        s if s.starts_with("daily") => ScheduleType::Daily,
        s if s.starts_with("weekly") => ScheduleType::Weekly,
        s if s.starts_with("monthly") => ScheduleType::Monthly,
        _ => ScheduleType::Yearly,
    };

    let time = Text::new("执行时间 (HH:MM):")
        .with_default(existing.map(|t| t.time.as_str()).unwrap_or("07:30"))
        .prompt()?;

    let zip_root = Confirm::new("打包根目录文件发送到企业微信?")
        .with_default(existing.map(|t| t.zip_root_files).unwrap_or(true))
        .prompt()?;

    let root_patterns = if zip_root {
        let current = existing.map(|t| t.root_file_patterns.clone())
            .unwrap_or_else(|| vec!["*.xlsx".to_string(), "*.xls".to_string()]);
        println!("当前文件匹配模式: {}", current.join(", "));
        let edit = Confirm::new("修改文件匹配模式?")
            .with_default(false)
            .prompt()?;
        if edit {
            let input = Text::new("文件匹配模式（逗号分隔，如 *.xlsx,*.xls,*.txt）:")
                .with_default(&current.join(","))
                .prompt()?;
            input.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
        } else {
            current
        }
    } else {
        vec![]
    };

    let incremental = Confirm::new("增量模式（只打包变化文件）?")
        .with_default(existing.map(|t| t.notify.incremental).unwrap_or(true))
        .prompt()?;

    let skip_first = Confirm::new("首次运行跳过文件发送?")
        .with_default(existing.map(|t| t.notify.skip_first_run).unwrap_or(true))
        .prompt()?;

    let recursive_send = Confirm::new("递归扫描子文件夹打包发送?（默认只发根目录文件）")
        .with_default(existing.map(|t| t.notify.recursive_send).unwrap_or(false))
        .prompt()?;

    let enabled = Confirm::new("启用该任务?")
        .with_default(existing.map(|t| t.enabled).unwrap_or(true))
        .prompt()?;

    // 排除模式
    let current_exclude = existing.map(|t| t.exclude.clone()).unwrap_or_default();
    let exclude = prompt_exclude_patterns(&current_exclude)?;

    Ok(TaskConfig {
        name,
        source,
        schedule,
        time,
        weekday: existing.and_then(|t| t.weekday),
        monthday: existing.and_then(|t| t.monthday),
        include: existing.map(|t| t.include.clone()).unwrap_or_default(),
        exclude,
        exclude_regex: existing.map(|t| t.exclude_regex.clone()).unwrap_or_default(),
        zip_root_files: zip_root,
        root_file_patterns: root_patterns,
        notify: TaskNotifyConfig {
            skip_first_run: skip_first,
            incremental,
            recursive_send,
        },
        enabled,
    })
}

fn prompt_exclude_patterns(current: &[String]) -> Result<Vec<String>> {
    if !current.is_empty() {
        println!("当前排除模式: {}", current.join(", "));
        let edit = Confirm::new("修改排除模式?")
            .with_default(false)
            .prompt()?;
        if !edit {
            return Ok(current.to_vec());
        }
    } else {
        let add = Confirm::new("添加文件排除模式?")
            .with_default(false)
            .prompt()?;
        if !add {
            return Ok(vec![]);
        }
    }

    let mut patterns = Vec::new();
    loop {
        let pat = Text::new("排除模式（glob，留空结束）:")
            .with_placeholder("例如：~$* 或 *.tmp")
            .with_default("")
            .prompt()?;
        if pat.is_empty() {
            break;
        }
        patterns.push(pat);
    }
    Ok(patterns)
}

// ═══════════════════════════════════════
// 输出
// ═══════════════════════════════════════

fn print_next_steps(output_path: &Path, is_edit: bool) {
    println!();
    if is_edit {
        println!("✅ 配置已更新: {}", output_path.display());
    } else {
        println!("✅ 配置已保存到: {}", output_path.display());
        println!();
        println!("下一步:");
        println!("  1. 初始化仓库:   rustic-backup.exe init");
        println!("  2. 注册计划任务: rustic-backup.exe install --time 07:30");
        println!("  3. 手动测试:     rustic-backup.exe run");
    }
}

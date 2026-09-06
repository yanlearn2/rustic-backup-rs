use anyhow::Result;
use chrono::{Datelike, Local, Weekday};
use std::path::{Path, PathBuf};

use crate::config::{Config, ScheduleType, TaskConfig};
use crate::logger::Logger;
use crate::notify::WechatNotifier;
use crate::rustic::{format_size, Rustic};
use crate::wedrive::WedriveUploader;
use crate::zip_util;

/// 单个任务执行结果
#[derive(Debug, Clone, Default)]
pub struct TaskResult {
    pub task_name: String,
    pub success: bool,
    pub snapshot_id: Option<String>,
    pub files_scanned: u64,
    pub files_size: u64,
    pub files_sent: u64,
    pub send_type: String,
    pub send_success: bool,
    pub wedrive_path: Option<String>,
    pub is_first_run: bool,
    pub changed_files: Vec<String>,
    pub error: Option<String>,
}

/// 备份调度器
pub struct Scheduler<'a> {
    config: &'a Config,
    rustic: Rustic<'a>,
    notifier: WechatNotifier,
    wedrive: WedriveUploader,
    logger: &'a Logger,
}

impl<'a> Scheduler<'a> {
    pub fn new(config: &'a Config, logger: &'a Logger) -> Self {
        let rustic = Rustic::new(config);
        let notifier = WechatNotifier::new(&config.notify.webhook);
        let wedrive = WedriveUploader::new(None, None, None); // 可从配置扩展
        Self {
            config,
            rustic,
            notifier,
            wedrive,
            logger,
        }
    }

    /// 判断任务是否应在今天执行
    pub fn should_run_today(task: &TaskConfig) -> bool {
        if !task.enabled {
            return false;
        }
        let now = Local::now();
        match task.schedule {
            ScheduleType::Daily => true,
            ScheduleType::Weekly => {
                let target_weekday = task.weekday.unwrap_or(6); // 默认周六
                let current_weekday = match now.weekday() {
                    Weekday::Sun => 0,
                    Weekday::Mon => 1,
                    Weekday::Tue => 2,
                    Weekday::Wed => 3,
                    Weekday::Thu => 4,
                    Weekday::Fri => 5,
                    Weekday::Sat => 6,
                };
                current_weekday == target_weekday
            }
            ScheduleType::Monthly => {
                let target_day = task.monthday.unwrap_or(1);
                now.day() == target_day
            }
            ScheduleType::Yearly => {
                now.month() == 1 && now.day() == 1
            }
        }
    }

    /// 执行所有到期任务
    pub fn run_all_due(&self, notify_enabled: bool, force_notify: bool) -> Result<Vec<TaskResult>> {
        let mut results = Vec::new();
        for task in &self.config.tasks {
            if Self::should_run_today(task) {
                self.logger.info(&format!("========== 开始执行任务: {} ==========", task.name));
                let result = self.run_task(task, notify_enabled, force_notify).unwrap_or_else(|e| {
                    self.logger.error(&format!("任务执行异常: {}", e));
                    TaskResult {
                        task_name: task.name.clone(),
                        success: false,
                        error: Some(e.to_string()),
                        ..Default::default()
                    }
                });
                self.logger.info(&format!("========== 任务完成: {} (成功={}) ==========", task.name, result.success));
                results.push(result);
            } else {
                self.logger.info(&format!("跳过任务: {} (今日不执行)", task.name));
            }
        }

        // 发送汇总报告
        if !results.is_empty() && notify_enabled {
            self.send_summary_report(&results)?;
        }

        Ok(results)
    }

    /// 执行指定任务（按序号，1-based）
    pub fn run_task_by_index(&self, idx: usize, notify_enabled: bool, force_notify: bool) -> Result<TaskResult> {
        let task = self.config.task_by_index(idx)
            .ok_or_else(|| anyhow::anyhow!("任务序号不存在: {}", idx))?;
        self.run_task(task, notify_enabled, force_notify)
    }

    pub fn run_task(&self, task: &TaskConfig, notify_enabled: bool, force_notify: bool) -> Result<TaskResult> {
        let mut result = TaskResult {
            task_name: task.name.clone(),
            success: false,
            snapshot_id: None,
            files_scanned: 0,
            files_size: 0,
            files_sent: 0,
            send_type: String::new(),
            send_success: false,
            wedrive_path: None,
            is_first_run: false,
            changed_files: Vec::new(),
            error: None,
        };

        // 检查源目录
        let source_path = Path::new(&task.source);
        if !source_path.exists() {
            let msg = format!("源目录不存在: {}", task.source);
            self.logger.error(&msg);
            result.error = Some(msg.clone());
            let _ = self.notifier.send_text(&format!("❌ 备份失败：{}\n任务: {}", msg, task.name));
            return Ok(result);
        }

        // 检查是否首次运行（该任务无历史快照）
        let previous_snapshot = self.rustic.latest_snapshot(task).unwrap_or(None);
        let is_first_run = previous_snapshot.is_none();
        result.is_first_run = is_first_run;

        // 1. 扫描根目录文件（用于打包发送）
        let mut root_files = if task.zip_root_files {
            zip_util::scan_root_files(source_path, &task.root_file_patterns, task.notify.recursive_send)?
        } else {
            Vec::new()
        };

        // 应用正则排除
        root_files = zip_util::filter_by_regex(root_files, &task.exclude_regex)?;

        result.files_scanned = root_files.len() as u64;
        result.files_size = root_files.iter().filter_map(|f| std::fs::metadata(f).ok().map(|m| m.len())).sum();

        // 2. 增量模式：只打包变化的文件
        let files_to_pack: Vec<PathBuf> = if task.notify.incremental && !is_first_run {
            if let Some(prev) = &previous_snapshot {
                // 先执行备份拿到新快照ID，再 diff
                // 但我们需要先知道变化文件才能打包，所以先做一个临时方案：
                // 直接用文件修改时间判断变化（比 rustic diff 简单，不依赖先备份）
                self.filter_changed_files(&root_files, prev)
            } else {
                root_files.clone()
            }
        } else {
            root_files.clone()
        };

        result.changed_files = files_to_pack.iter()
            .filter_map(|f| f.file_name().and_then(|n| n.to_str()).map(|s| s.to_string()))
            .collect();
        result.files_sent = files_to_pack.len() as u64;

        // 3. 打包并发送（首次运行且 skip_first_run 时跳过）
        let skip_send = !notify_enabled || (!force_notify && is_first_run && task.notify.skip_first_run);
        if !skip_send && !files_to_pack.is_empty() {
            self.pack_and_send(task, &files_to_pack, &mut result)?;
        } else if !notify_enabled {
            self.logger.info("通知已禁用，跳过文件发送");
            result.send_type = "通知禁用".to_string();
            result.send_success = true;
        } else if skip_send {
            self.logger.info("首次运行，跳过文件发送");
            result.send_type = "首次运行跳过".to_string();
        } else if files_to_pack.is_empty() {
            self.logger.info("无变化文件，跳过打包发送");
            result.send_type = "无变化".to_string();
            result.send_success = true;
        }

        // 4. 执行 rustic 备份（整个目录）
        self.logger.info(&format!("执行 rustic 备份: {}", task.source));
        match self.rustic.backup(task, &[]) {
            Ok(snapshot_id) => {
                self.logger.info(&format!("备份成功，快照ID: {}", snapshot_id));
                result.snapshot_id = Some(snapshot_id);
                result.success = true;
            }
            Err(e) => {
                let msg = format!("rustic 备份失败: {}", e);
                self.logger.error(&msg);
                result.error = Some(msg);
                result.success = false;
            }
        }

        Ok(result)
    }

    /// 根据上一次快照的时间过滤变化文件
    fn filter_changed_files(&self, files: &[PathBuf], prev_snapshot: &crate::rustic::Snapshot) -> Vec<PathBuf> {
        // 解析快照时间
        let snapshot_time = chrono::DateTime::parse_from_rfc3339(&prev_snapshot.time)
            .map(|dt| dt.with_timezone(&Local))
            .unwrap_or_else(|_| Local::now());

        files.iter()
            .filter(|f| {
                if let Ok(meta) = std::fs::metadata(f) {
                    if let Ok(modified) = meta.modified() {
                        let modified_time: chrono::DateTime<Local> = modified.into();
                        return modified_time > snapshot_time;
                    }
                }
                true // 无法判断修改时间的文件保留
            })
            .cloned()
            .collect()
    }

    /// 打包文件并发送到企业微信
    fn pack_and_send(&self, task: &TaskConfig, files: &[PathBuf], result: &mut TaskResult) -> Result<()> {
        let temp_dir = self.config.temp_dir().join(format!("{}_staging", task.name));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir)?;

        // 复制文件到临时目录（解决文件占用）
        let copied_files = zip_util::copy_files_to_temp(files, &temp_dir)?;
        if copied_files.is_empty() {
            self.logger.warn("没有可复制的文件，跳过发送");
            result.send_type = "无文件可发".to_string();
            return Ok(());
        }

        let date_str = Local::now().format("%Y%m%d").to_string();
        let suffix = if task.notify.incremental && !result.is_first_run { "增量" } else { "全量" };
        let base_name = format!("{}_{}_{}", task.name, date_str, suffix);

        // 计算总大小
        let total_size: u64 = copied_files.iter()
            .filter_map(|f| std::fs::metadata(f).ok().map(|m| m.len()))
            .sum();

        let max_size = self.config.notify.max_webhook_size;
        let need_split = self.config.notify.split_large_files && total_size > max_size;

        if need_split {
            // 分片模式
            self.logger.info(&format!("总大小 {} 超过 {}，启动分片发送", format_size(total_size), format_size(max_size)));
            self.send_split_parts(&copied_files, &temp_dir, &base_name, result)?;
        } else {
            // 单包模式
            let zip_name = format!("{}.zip", base_name);
            let zip_path = self.config.temp_dir().join(&zip_name);
            self.logger.info(&format!("打包 {} 个文件到 {}", copied_files.len(), zip_name));
            zip_util::zip_files(&copied_files, &zip_path, Some(&temp_dir))?;
            let zip_size = std::fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0);
            self.logger.info(&format!("打包完成: {}", format_size(zip_size)));
            self.send_single_zip(&zip_path, zip_size, result)?;
            let _ = std::fs::remove_file(&zip_path);
        }

        // 清理临时 staging 目录
        let _ = std::fs::remove_dir_all(&temp_dir);

        Ok(())
    }

    /// 单包发送（webhook 直发 or 微盘 or 跳过）
    fn send_single_zip(&self, zip_path: &Path, zip_size: u64, result: &mut TaskResult) -> Result<()> {
        if zip_size <= self.config.notify.max_webhook_size {
            self.logger.info("zip ≤限制，webhook 直发");
            result.send_type = "webhook直发".to_string();
            match self.notifier.send_file(zip_path) {
                Ok(_) => {
                    self.logger.info("文件发送成功");
                    result.send_success = true;
                }
                Err(e) => {
                    self.logger.error(&format!("文件发送失败: {}", e));
                    result.send_success = false;
                    result.error = Some(format!("发送失败: {}", e));
                }
            }
        } else if self.config.notify.wedrive_enabled {
            self.logger.warn("zip 超限制，上传微盘");
            result.send_type = "微盘上传".to_string();
            match self.wedrive.upload(zip_path) {
                Ok(wresult) => {
                    if wresult.success {
                        self.logger.info(&format!("微盘上传成功: {:?}", wresult.file_path));
                        result.send_success = true;
                        result.wedrive_path = wresult.file_path;
                    } else {
                        self.logger.error(&format!("微盘上传失败: {:?}", wresult.error));
                        result.send_success = false;
                        result.error = wresult.error;
                    }
                }
                Err(e) => {
                    self.logger.error(&format!("微盘上传异常: {}", e));
                    result.send_success = false;
                    result.error = Some(format!("微盘异常: {}", e));
                }
            }
        } else {
            self.logger.warn("文件超限制且微盘未启用，跳过发送");
            result.send_type = "超限跳过".to_string();
        }
        Ok(())
    }

    /// 查找 7za.exe（优先程序目录，其次系统 PATH）
    fn find_7za(&self) -> Option<PathBuf> {
        // 1. exe 所在目录的 bin/7za.exe
        if let Some(exe_dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        {
            let local = exe_dir.join("bin").join("7za.exe");
            if local.exists() {
                return Some(local);
            }
        }
        // 2. 系统 PATH
        for name in &["7za.exe", "7z.exe"] {
            if let Ok(output) = std::process::Command::new("where")
                .arg(name)
                .output()
            {
                if output.status.success() {
                    if let Some(path) = String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .next()
                        .map(|s| PathBuf::from(s.trim()))
                    {
                        if path.exists() {
                            return Some(path);
                        }
                    }
                }
            }
        }
        None
    }

    /// 用 7za 创建标准分卷 ZIP 并逐个发送
    fn send_with_7z_split(&self, seven_zip: &Path, files: &[PathBuf], _temp_dir: &Path, base_name: &str, result: &mut TaskResult) -> Result<()> {
        use std::process::Command;

        let part_size = self.config.notify.split_part_size;
        let out_zip = self.config.temp_dir().join(format!("{}.zip", base_name));

        // 清理旧分卷
        let _ = std::fs::remove_file(&out_zip);
        for i in 1..100 {
            let _ = std::fs::remove_file(self.config.temp_dir().join(format!("{}.z{:02}", base_name, i)));
        }

        self.logger.info(&format!("用 7za 创建分卷 ZIP，单卷上限 {}", format_size(part_size)));

        let mut cmd = Command::new(seven_zip);
        cmd.arg("a")
            .arg("-tzip")
            .arg(format!("-v{}", part_size))
            .arg("-mx=1") // 快速压缩
            .arg(&out_zip);
        for f in files {
            cmd.arg(f);
        }

        let output = cmd.output()?;
        if !output.status.success() {
            anyhow::bail!("7za 执行失败: {}", String::from_utf8_lossy(&output.stderr));
        }

        // 收集分卷文件
        let mut parts: Vec<PathBuf> = Vec::new();
        if out_zip.exists() {
            parts.push(out_zip.clone());
        }
        for i in 1..100 {
            let part = self.config.temp_dir().join(format!("{}.z{:02}", base_name, i));
            if part.exists() {
                parts.push(part);
            } else {
                break;
            }
        }

        if parts.is_empty() {
            anyhow::bail!("7za 未生成分卷文件");
        }

        self.logger.info(&format!("生成 {} 个分卷", parts.len()));

        // 逐个发送
        let mut all_ok = true;
        let mut sent = 0;
        for (i, part) in parts.iter().enumerate() {
            let size = std::fs::metadata(part)?.len();
            self.logger.info(&format!("分卷 {}/{}: {}", i + 1, parts.len(), format_size(size)));

            match self.notifier.send_file(part) {
                Ok(_) => {
                    self.logger.info(&format!("分卷 {} 发送成功", i + 1));
                    sent += 1;
                }
                Err(e) => {
                    self.logger.error(&format!("分卷 {} 发送失败: {}", i + 1, e));
                    all_ok = false;
                    result.error = Some(format!("分卷{}发送失败: {}", i + 1, e));
                }
            }
            let _ = std::fs::remove_file(part);
        }

        result.send_type = format!("7z分卷({}/{})", sent, parts.len());
        result.send_success = all_ok;
        Ok(())
    }

    /// 分片发送：按文件大小分组，每组打一个 zip，逐个 webhook 发送
    fn send_split_parts(&self, files: &[PathBuf], temp_dir: &Path, base_name: &str, result: &mut TaskResult) -> Result<()> {
        // 优先用 7za 创建标准分卷 ZIP
        if let Some(seven_zip) = self.find_7za() {
            match self.send_with_7z_split(&seven_zip, files, temp_dir, base_name, result) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    self.logger.error(&format!("7za 分卷失败({})，回退到内置分片", e));
                }
            }
        }

        let part_size = self.config.notify.split_part_size;

        // 按文件大小降序排序（贪心装箱，大的先放）
        let mut files_with_size: Vec<(PathBuf, u64)> = files.iter()
            .map(|f| (f.clone(), std::fs::metadata(f).ok().map(|m| m.len()).unwrap_or(0)))
            .collect();
        files_with_size.sort_by(|a, b| b.1.cmp(&a.1));

        // 贪心分组
        let mut groups: Vec<Vec<PathBuf>> = Vec::new();
        let mut group_sizes: Vec<u64> = Vec::new();
        for (file, size) in &files_with_size {
            let mut placed = false;
            for i in 0..groups.len() {
                if group_sizes[i] + size <= part_size {
                    groups[i].push(file.clone());
                    group_sizes[i] += size;
                    placed = true;
                    break;
                }
            }
            if !placed {
                groups.push(vec![file.clone()]);
                group_sizes.push(*size);
            }
        }

        self.logger.info(&format!("分为 {} 个分片，单包上限 {}", groups.len(), format_size(part_size)));

        let mut all_success = true;
        let mut sent_count = 0;

        for (i, group) in groups.iter().enumerate() {
            let part_num = i + 1;
            let zip_name = format!("{}_part{}.zip", base_name, part_num);
            let zip_path = self.config.temp_dir().join(&zip_name);

            self.logger.info(&format!("打包分片 {}/{} ({} 个文件)", part_num, groups.len(), group.len()));
            zip_util::zip_files(group, &zip_path, Some(temp_dir))?;
            let zip_size = std::fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0);
            self.logger.info(&format!("分片 {} 大小: {}", part_num, format_size(zip_size)));

            let max_size = self.config.notify.max_webhook_size;

            if zip_size > max_size {
                // 单个分片仍超限，进行二进制拆分
                self.logger.info(&format!("分片 {} 超过 {}，进行二进制拆分...", part_num, format_size(max_size)));
                let ok = self.split_and_send_file(&zip_path, &zip_name, max_size, part_num, groups.len())?;
                if !ok {
                    all_success = false;
                    result.error = Some(format!("分片{}拆分后发送失败", part_num));
                } else {
                    sent_count += 1;
                }
            } else {
                match self.notifier.send_file(&zip_path) {
                    Ok(_) => {
                        self.logger.info(&format!("分片 {} 发送成功", part_num));
                        sent_count += 1;
                    }
                    Err(e) => {
                        self.logger.error(&format!("分片 {} 发送失败: {}", part_num, e));
                        all_success = false;
                        result.error = Some(format!("分片{}发送失败: {}", part_num, e));
                    }
                }
            }

            let _ = std::fs::remove_file(&zip_path);
        }

        result.send_type = format!("webhook分片({}/{})", sent_count, groups.len());
        result.send_success = all_success;

        Ok(())
    }

    /// 二进制拆分大文件并逐个发送
    fn split_and_send_file(&self, file_path: &Path, base_name: &str, max_size: u64, part_num: usize, total_parts: usize) -> Result<bool> {
        use std::io::{Read, Write};

        let chunk_size = max_size - 1024 * 1024; // 留1MB余量
        let mut file = std::fs::File::open(file_path)?;
        let file_size = file.metadata()?.len();
        let total_chunks = (file_size + chunk_size - 1) / chunk_size;

        self.logger.info(&format!("拆分为 {} 个二进制块，每块上限 {}", total_chunks, format_size(chunk_size)));

        let mut buffer = vec![0u8; chunk_size as usize];
        let mut all_ok = true;

        for i in 0..total_chunks {
            let n = file.read(&mut buffer)?;
            if n == 0 { break; }

            let chunk_name = format!("{}_chunk{}_{}.bin", base_name, i + 1, total_chunks);
            let chunk_path = self.config.temp_dir().join(&chunk_name);
            {
                let mut chunk_file = std::fs::File::create(&chunk_path)?;
                chunk_file.write_all(&buffer[..n])?;
            }

            let chunk_size_actual = std::fs::metadata(&chunk_path)?.len();
            self.logger.info(&format!("  二进制块 {}/{}: {}", i + 1, total_chunks, format_size(chunk_size_actual)));

            match self.notifier.send_file(&chunk_path) {
                Ok(_) => self.logger.info(&format!("  二进制块 {}/{} 发送成功", i + 1, total_chunks)),
                Err(e) => {
                    self.logger.error(&format!("  二进制块 {}/{} 发送失败: {}", i + 1, total_chunks, e));
                    all_ok = false;
                }
            }

            let _ = std::fs::remove_file(&chunk_path);
        }

        // 发送合并说明
        if all_ok {
            let readme_name = format!("{}_合并说明.txt", base_name);
            let readme_path = self.config.temp_dir().join(&readme_name);
            let readme_content = format!(
                "文件拆分合并说明\n================\n\n原文件: {}\n大小: {}\n拆分块数: {}\n\n合并方法（Windows命令行）:\n  copy /b {}_chunk1_*.bin + {}_chunk2_*..bin + ... {}\n\n或使用 PowerShell:\n  Get-Content {}_chunk*.bin -Encoding Byte | Set-Content {} -Encoding Byte\n\n注意: 必须按序号顺序合并，合并后文件为 zip 格式，可直接解压。",
                base_name, format_size(file_size), total_chunks,
                base_name, base_name, base_name,
                base_name, base_name
            );
            std::fs::write(&readme_path, readme_content)?;
            let _ = self.notifier.send_file(&readme_path);
            let _ = std::fs::remove_file(&readme_path);
        }

        Ok(all_ok)
    }

    /// 发送汇总报告
    fn send_summary_report(&self, results: &[TaskResult]) -> Result<()> {
        let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let total_files: u64 = results.iter().map(|r| r.files_scanned).sum();
        let total_size: u64 = results.iter().map(|r| r.files_size).sum();
        let all_success = results.iter().all(|r| r.success);

        let status_emoji = if all_success { "✅" } else { "⚠️" };
        let mut lines = Vec::new();
        lines.push(format!("### {} 备份报告", status_emoji));
        lines.push(String::new());
        lines.push(format!("**📅 备份时间**: {}", now));
        lines.push(String::new());
        lines.push("**📊 备份统计**".to_string());
        lines.push(format!("> 扫描文件: {} 个", total_files));
        lines.push(format!("> 总大小: {}", format_size(total_size)));
        lines.push(String::new());
        lines.push("**📂 各任务详情**".to_string());

        for r in results {
            let snap_short = r.snapshot_id.as_ref()
                .map(|s| if s.len() >= 8 { &s[..8] } else { s })
                .unwrap_or("无");
            let send_emoji = if r.send_success { "✅" } else { "❌" };
            let status = if r.success { "成功" } else { "失败" };

            lines.push(format!("> **{}**: {} ({}个/{})", r.task_name, status, r.files_scanned, format_size(r.files_size)));
            lines.push(format!(">   发送: {} {} | 快照: {}", r.send_type, send_emoji, snap_short));

            if r.is_first_run {
                lines.push(">   ℹ️ 首次运行".to_string());
            }
            if !r.changed_files.is_empty() && r.changed_files.len() <= 10 {
                lines.push(format!(">   🔄 变化文件: {}", r.changed_files.join(", ")));
            } else if r.changed_files.len() > 10 {
                lines.push(format!(">   🔄 变化文件: {} 个", r.changed_files.len()));
            }
            if let Some(wp) = &r.wedrive_path {
                lines.push(format!(">   📁 微盘: {}", wp));
            }
            if let Some(err) = &r.error {
                lines.push(format!(">   ❌ 错误: {}", err));
            }
        }

        lines.push(String::new());
        lines.push("---".to_string());
        lines.push("*自动备份任务 · rustic-backup-rs*".to_string());

        let markdown = lines.join("\n");
        match self.notifier.send_markdown(&markdown) {
            Ok(_) => self.logger.info("汇总报告发送成功"),
            Err(e) => self.logger.error(&format!("汇总报告发送失败: {}", e)),
        }

        Ok(())
    }
}

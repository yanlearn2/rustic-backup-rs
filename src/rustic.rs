use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{Config, RetentionConfig, TaskConfig};

/// rustic 快照信息
#[derive(Debug, Clone, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub time: String,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub summary: Option<SnapshotSummary>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SnapshotSummary {
    #[serde(rename = "files_new")]
    pub files_new: Option<u64>,
    #[serde(rename = "files_total")]
    pub files_total: Option<u64>,
    #[serde(rename = "bytes_processed")]
    pub bytes_processed: Option<u64>,
}

/// rustic diff 结果中的变化文件
#[derive(Debug, Clone)]
pub struct DiffFile {
    pub path: String,
    pub change_type: DiffChangeType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffChangeType {
    Added,
    Modified,
    Removed,
}

/// 仓库统计信息
#[derive(Debug, Clone, Default)]
pub struct RepoStats {
    pub total_size: u64,
    pub snapshot_count: u64,
}

/// rustic 命令封装
pub struct Rustic<'a> {
    config: &'a Config,
    repo_path: PathBuf,
}

impl<'a> Rustic<'a> {
    pub fn new(config: &'a Config) -> Self {
        let repo_path = PathBuf::from(&config.repo.path);
        Self { config, repo_path }
    }

    fn base_cmd(&self) -> Command {
        let mut cmd = Command::new(self.config.rustic_exe());
        cmd.env("RUSTIC_PASSWORD", &self.config.repo.password);
        cmd.arg("--repo").arg(&self.repo_path);
        cmd
    }

    fn run_cmd(cmd: &mut Command) -> Result<(String, String, i32)> {
        let output = cmd.output().context("执行 rustic 命令失败")?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let code = output.status.code().unwrap_or(-1);
        Ok((stdout, stderr, code))
    }

    /// 初始化仓库
    pub fn init(&self) -> Result<()> {
        if self.repo_path.exists() {
            return Err(anyhow!("仓库已存在: {}", self.repo_path.display()));
        }
        let mut cmd = self.base_cmd();
        cmd.arg("init");
        let (stdout, stderr, code) = Self::run_cmd(&mut cmd)?;
        if code != 0 {
            return Err(anyhow!("初始化仓库失败 (exit {}):\n{}\n{}", code, stdout, stderr));
        }
        Ok(())
    }

    /// 执行备份，返回快照ID
    pub fn backup(&self, task: &TaskConfig, extra_args: &[&str]) -> Result<String> {
        let mut cmd = self.base_cmd();
        cmd.arg("backup");
        cmd.arg(&task.source);

        // 标签
        cmd.arg("--tag").arg(task.schedule.as_tag());
        cmd.arg("--tag").arg(&task.name);

        // 额外参数
        for a in extra_args {
            cmd.arg(a);
        }

        let (stdout, stderr, code) = Self::run_cmd(&mut cmd)?;
        if code != 0 {
            return Err(anyhow!("备份失败 (exit {}):\n{}\n{}", code, stdout, stderr));
        }

        // 解析快照ID: snapshot <id> successfully saved
        let snapshot_id = stdout
            .lines()
            .chain(stderr.lines())
            .find_map(|line| {
                let re = regex::Regex::new(r"snapshot ([a-f0-9]+) successfully saved").ok()?;
                re.captures(line).and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
            })
            .ok_or_else(|| anyhow!("无法解析快照ID，输出:\n{}\n{}", stdout, stderr))?;

        Ok(snapshot_id)
    }

    /// 列出快照（rustic 不支持 --tag 过滤，在应用层过滤）
    pub fn snapshots(&self, tags: &[&str]) -> Result<Vec<Snapshot>> {
        let mut cmd = self.base_cmd();
        cmd.arg("snapshots").arg("--json");

        let (stdout, stderr, code) = Self::run_cmd(&mut cmd)?;
        if code != 0 {
            return Err(anyhow!("列出快照失败 (exit {}):\n{}", code, stderr));
        }
        let mut snapshots: Vec<Snapshot> = serde_json::from_str(&stdout)
            .with_context(|| format!("解析快照JSON失败: {}", stdout))?;

        // 应用层按 tag 过滤（所有 tag 都必须匹配，AND 逻辑）
        if !tags.is_empty() {
            snapshots.retain(|s| {
                tags.iter().all(|t| s.tags.iter().any(|st| st == *t))
            });
        }

        Ok(snapshots)
    }

    /// 获取某个任务的最新快照
    pub fn latest_snapshot(&self, task: &TaskConfig) -> Result<Option<Snapshot>> {
        let snapshots = self.snapshots(&[task.schedule.as_tag(), &task.name])?;
        // 按时间倒序，取最新
        let mut sorted = snapshots;
        sorted.sort_by(|a, b| b.time.cmp(&a.time));
        Ok(sorted.into_iter().next())
    }

    /// 恢复快照
    pub fn restore(&self, snapshot_id: &str, target: &Path, include: Option<&str>) -> Result<()> {
        let mut cmd = self.base_cmd();
        cmd.arg("restore").arg(snapshot_id).arg("--target").arg(target);
        if let Some(inc) = include {
            cmd.arg("--include").arg(inc);
        }
        let (stdout, stderr, code) = Self::run_cmd(&mut cmd)?;
        if code != 0 {
            return Err(anyhow!("恢复失败 (exit {}):\n{}\n{}", code, stdout, stderr));
        }
        Ok(())
    }

    /// 清理旧快照（forget + prune）
    pub fn forget_and_prune(&self, retention: &RetentionConfig, tag: Option<&str>, dry_run: bool) -> Result<String> {
        let mut cmd = self.base_cmd();
        cmd.arg("forget");
        cmd.arg("--keep-daily").arg(retention.daily.to_string());
        cmd.arg("--keep-weekly").arg(retention.weekly.to_string());
        cmd.arg("--keep-monthly").arg(retention.monthly.to_string());
        cmd.arg("--keep-yearly").arg(retention.yearly.to_string());
        if let Some(t) = tag {
            cmd.arg("--tag").arg(t);
        }
        if dry_run {
            cmd.arg("--dry-run");
        } else {
            cmd.arg("--prune");
        }

        let (stdout, stderr, code) = Self::run_cmd(&mut cmd)?;
        if code != 0 {
            return Err(anyhow!("清理失败 (exit {}):\n{}\n{}", code, stdout, stderr));
        }
        Ok(format!("{}\n{}", stdout, stderr))
    }

    /// 校验仓库完整性
    pub fn check(&self) -> Result<()> {
        let mut cmd = self.base_cmd();
        cmd.arg("check");
        let (stdout, stderr, code) = Self::run_cmd(&mut cmd)?;
        if code != 0 {
            return Err(anyhow!("仓库校验失败 (exit {}):\n{}\n{}", code, stdout, stderr));
        }
        Ok(())
    }

    /// 仓库统计信息
    pub fn repoinfo(&self) -> Result<RepoStats> {
        let mut cmd = self.base_cmd();
        cmd.arg("repoinfo").arg("--json");
        let (stdout, stderr, code) = Self::run_cmd(&mut cmd)?;
        if code != 0 {
            // repoinfo --json 可能不支持，降级为解析文本
            return self.repoinfo_fallback(&stdout, &stderr);
        }
        // 尝试解析 JSON
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&stdout) {
            let total_size = v
                .get("total_size")
                .or_else(|| v.get("size"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let snapshot_count = v
                .get("snapshot_count")
                .or_else(|| v.get("snapshots"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            return Ok(RepoStats { total_size, snapshot_count });
        }
        self.repoinfo_fallback(&stdout, &stderr)
    }

    fn repoinfo_fallback(&self, stdout: &str, stderr: &str) -> Result<RepoStats> {
        let combined = format!("{}\n{}", stdout, stderr);
        let mut stats = RepoStats::default();

        // 尝试从文本中提取大小
        for line in combined.lines() {
            let lower = line.to_lowercase();
            if lower.contains("total size") || lower.contains("repo size") {
                if let Some(num) = extract_number_from_line(line) {
                    stats.total_size = num;
                }
            }
            if lower.contains("snapshot") && lower.contains("count") {
                if let Some(num) = extract_number_from_line(line) {
                    stats.snapshot_count = num;
                }
            }
        }

        // 快照数从 snapshots 命令获取
        if stats.snapshot_count == 0 {
            if let Ok(snaps) = self.snapshots(&[]) {
                stats.snapshot_count = snaps.len() as u64;
            }
        }

        Ok(stats)
    }

    /// 对比两个快照，返回变化文件列表
    pub fn diff(&self, old_id: &str, new_id: &str) -> Result<Vec<DiffFile>> {
        let mut cmd = self.base_cmd();
        cmd.arg("diff").arg(old_id).arg(new_id);
        let (stdout, stderr, code) = Self::run_cmd(&mut cmd)?;
        if code != 0 {
            return Err(anyhow!("diff 失败 (exit {}):\n{}", code, stderr));
        }

        let mut files = Vec::new();
        let combined = format!("{}\n{}", stdout, stderr);

        for line in combined.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // rustic diff 输出格式通常是: + 新增文件 / M 修改文件 / - 删除文件
            // 也可能是: added: / modified: / removed:
            let lower = line.to_lowercase();
            if let Some(rest) = line.strip_prefix('+') {
                files.push(DiffFile { path: rest.trim().to_string(), change_type: DiffChangeType::Added });
            } else if let Some(rest) = line.strip_prefix('M') {
                files.push(DiffFile { path: rest.trim().to_string(), change_type: DiffChangeType::Modified });
            } else if let Some(rest) = line.strip_prefix('-') {
                files.push(DiffFile { path: rest.trim().to_string(), change_type: DiffChangeType::Removed });
            } else if lower.starts_with("added:") {
                files.push(DiffFile { path: line[6..].trim().to_string(), change_type: DiffChangeType::Added });
            } else if lower.starts_with("modified:") {
                files.push(DiffFile { path: line[9..].trim().to_string(), change_type: DiffChangeType::Modified });
            } else if lower.starts_with("removed:") {
                files.push(DiffFile { path: line[8..].trim().to_string(), change_type: DiffChangeType::Removed });
            }
        }

        Ok(files)
    }
}

fn extract_number_from_line(line: &str) -> Option<u64> {
    // 提取行中的数字（支持逗号分隔）
    let cleaned: String = line.chars().filter(|c| c.is_ascii_digit()).collect();
    cleaned.parse().ok()
}

/// 格式化文件大小
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    format!("{:.2} {}", size, UNITS[unit_idx])
}

/// 判断快照属于哪个保留层级
pub fn retention_tier(snapshot: &Snapshot) -> &'static str {
    for tag in &snapshot.tags {
        match tag.as_str() {
            "daily" => return "每日",
            "weekly" => return "每周",
            "monthly" => return "每月",
            "yearly" => return "每年",
            _ => {}
        }
    }
    "未知"
}

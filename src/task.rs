use anyhow::Result;
use std::path::Path;

use crate::config::Config;

/// 任务管理（enable/disable，修改配置文件）
pub struct TaskManager;

impl TaskManager {
    /// 启用任务
    pub fn enable(config_path: &Path, task_index: usize) -> Result<()> {
        let mut config = Config::load(config_path)?;
        let task = config.tasks.get_mut(task_index - 1)
            .ok_or_else(|| anyhow::anyhow!("任务序号不存在: {}", task_index))?;
        task.enabled = true;
        config.save(config_path)?;
        Ok(())
    }

    /// 禁用任务
    pub fn disable(config_path: &Path, task_index: usize) -> Result<()> {
        let mut config = Config::load(config_path)?;
        let task = config.tasks.get_mut(task_index - 1)
            .ok_or_else(|| anyhow::anyhow!("任务序号不存在: {}", task_index))?;
        task.enabled = false;
        config.save(config_path)?;
        Ok(())
    }

    /// 列出所有任务状态
    pub fn list(config: &Config) -> Vec<TaskInfo> {
        config.tasks.iter().enumerate().map(|(i, t)| {
            TaskInfo {
                index: i + 1,
                name: t.name.clone(),
                source: t.source.clone(),
                schedule: format!("{:?}", t.schedule),
                time: t.time.clone(),
                enabled: t.enabled,
                include: t.include.clone(),
                exclude: t.exclude.clone(),
                zip_root_files: t.zip_root_files,
                skip_first_run: t.notify.skip_first_run,
                incremental: t.notify.incremental,
            }
        }).collect()
    }
}

#[derive(Debug, Clone)]
pub struct TaskInfo {
    pub index: usize,
    pub name: String,
    pub source: String,
    pub schedule: String,
    pub time: String,
    pub enabled: bool,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub zip_root_files: bool,
    pub skip_first_run: bool,
    pub incremental: bool,
}

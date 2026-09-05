use anyhow::{anyhow, Context, Result};
use std::process::Command;

/// 解码 Windows GBK 输出为 UTF-8 字符串
fn decode_gbk(bytes: &[u8]) -> String {
    let (cow, _, _) = encoding_rs::GBK.decode(bytes);
    cow.to_string()
}

/// Windows 计划任务管理
pub struct TaskScheduler {
    task_name: String,
}

impl TaskScheduler {
    pub fn new(task_name: &str) -> Self {
        Self {
            task_name: task_name.to_string(),
        }
    }

    /// 注册计划任务（每天指定时间执行）
    pub fn install(&self, exe_path: &str, time: &str) -> Result<()> {
        if self.is_installed()? {
            return Err(anyhow!("计划任务已存在: {}（请先 uninstall）", self.task_name));
        }

        let output = Command::new("schtasks")
            .args([
                "/Create",
                "/TN", &self.task_name,
                "/TR", &format!("\"{}\" run", exe_path),
                "/SC", "DAILY",
                "/ST", time,
                "/F",
            ])
            .output()
            .context("执行 schtasks 失败")?;

        if !output.status.success() {
            let stderr = decode_gbk(&output.stderr);
            let stdout = decode_gbk(&output.stdout);
            return Err(anyhow!("创建计划任务失败: {}{}", stderr, stdout));
        }

        Ok(())
    }

    /// 移除计划任务
    pub fn uninstall(&self) -> Result<()> {
        if !self.is_installed()? {
            return Err(anyhow!("计划任务不存在: {}", self.task_name));
        }

        let output = Command::new("schtasks")
            .args(["/Delete", "/TN", &self.task_name, "/F"])
            .output()
            .context("执行 schtasks 失败")?;

        if !output.status.success() {
            let stderr = decode_gbk(&output.stderr);
            return Err(anyhow!("删除计划任务失败: {}", stderr));
        }

        Ok(())
    }

    /// 检查是否已安装
    pub fn is_installed(&self) -> Result<bool> {
        let output = Command::new("schtasks")
            .args(["/Query", "/TN", &self.task_name, "/FO", "CSV"])
            .output()
            .context("执行 schtasks 失败")?;
        Ok(output.status.success())
    }

    /// 获取任务状态信息
    pub fn status(&self) -> Result<TaskStatus> {
        let output = Command::new("schtasks")
            .args(["/Query", "/TN", &self.task_name, "/FO", "CSV", "/V"])
            .output()
            .context("执行 schtasks 失败")?;

        if !output.status.success() {
            return Err(anyhow!("查询计划任务失败: {}", self.task_name));
        }

        let stdout = decode_gbk(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();

        // CSV 格式，第二行是数据
        if lines.len() < 2 {
            return Err(anyhow!("无法解析计划任务信息"));
        }

        // 简单解析 CSV（不处理引号内逗号的复杂情况）
        let fields: Vec<&str> = lines[1].split(',').collect();

        Ok(TaskStatus {
            name: self.task_name.clone(),
            status: fields.get(1).map(|s| s.trim_matches('"').to_string()).unwrap_or_default(),
            next_run: fields.get(2).map(|s| s.trim_matches('"').to_string()).unwrap_or_default(),
            last_run: fields.get(4).map(|s| s.trim_matches('"').to_string()).unwrap_or_default(),
            last_result: fields.get(5).map(|s| s.trim_matches('"').to_string()).unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct TaskStatus {
    pub name: String,
    pub status: String,
    pub next_run: String,
    pub last_run: String,
    pub last_result: String,
}

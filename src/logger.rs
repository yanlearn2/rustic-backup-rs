use chrono::Local;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// 简单日志记录器
pub struct Logger {
    log_dir: PathBuf,
    current_file: Mutex<Option<(String, PathBuf)>>,
}

impl Logger {
    pub fn new(log_dir: PathBuf) -> Self {
        if !log_dir.exists() {
            let _ = fs::create_dir_all(&log_dir);
        }
        Self {
            log_dir,
            current_file: Mutex::new(None),
        }
    }

    fn log_file_for_today(&self) -> PathBuf {
        let today = Local::now().format("%Y%m%d").to_string();
        self.log_dir.join(format!("backup-{}.log", today))
    }

    pub fn info(&self, msg: &str) {
        self.write("INFO", msg);
    }

    pub fn warn(&self, msg: &str) {
        self.write("WARN", msg);
    }

    pub fn error(&self, msg: &str) {
        self.write("ERROR", msg);
    }

    fn write(&self, level: &str, msg: &str) {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let line = format!("[{}] [{}] {}\n", timestamp, level, msg);

        // 输出到控制台
        print!("{}", line);

        // 写入文件
        let log_file = self.log_file_for_today();
        if let Ok(mut guard) = self.current_file.lock() {
            // 检查是否需要切换文件（跨天）
            let need_new = match &*guard {
                Some((date, _)) => date != &Local::now().format("%Y%m%d").to_string(),
                None => true,
            };
            if need_new {
                *guard = Some((Local::now().format("%Y%m%d").to_string(), log_file.clone()));
            }
        }

        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)
        {
            let _ = f.write_all(line.as_bytes());
        }
    }

    /// 读取今天的日志内容
    pub fn read_today(&self) -> String {
        let log_file = self.log_file_for_today();
        fs::read_to_string(&log_file).unwrap_or_else(|_| "今日暂无日志".to_string())
    }

    /// 读取指定日期的日志
    pub fn read_date(&self, date: &str) -> String {
        let log_file = self.log_dir.join(format!("backup-{}.log", date));
        fs::read_to_string(&log_file).unwrap_or_else(|_| format!("无日志: {}", date))
    }
}

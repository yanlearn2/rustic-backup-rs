use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::process::Command;

/// 微盘上传结果
#[derive(Debug, Clone)]
pub struct WedriveResult {
    pub success: bool,
    pub file_path: Option<String>,
    pub file_id: Option<String>,
    pub error: Option<String>,
}

/// 微盘上传器
/// 优先使用 wecom-cli 命令行（如果系统已安装），
/// 否则降级为直接调用企业微信微盘 API（需要配置 corpid/secret）
pub struct WedriveUploader {
    wecom_cli_path: Option<String>,
    corpid: Option<String>,
    corpsecret: Option<String>,
}

impl WedriveUploader {
    pub fn new(wecom_cli_path: Option<String>, corpid: Option<String>, corpsecret: Option<String>) -> Self {
        Self {
            wecom_cli_path,
            corpid,
            corpsecret,
        }
    }

    /// 检测 wecom-cli 是否可用
    pub fn wecom_cli_available(&self) -> bool {
        if let Some(p) = &self.wecom_cli_path {
            return Path::new(p).exists();
        }
        // 尝试从 PATH 查找
        Command::new("wecom-cli")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// 上传文件到微盘
    pub fn upload(&self, file_path: &Path) -> Result<WedriveResult> {
        if !file_path.exists() {
            return Err(anyhow!("文件不存在: {}", file_path.display()));
        }

        if self.wecom_cli_available() {
            self.upload_via_cli(file_path)
        } else if self.corpid.is_some() && self.corpsecret.is_some() {
            self.upload_via_api(file_path)
        } else {
            Err(anyhow!(
                "微盘上传不可用：未找到 wecom-cli，且未配置 corpid/corpsecret"
            ))
        }
    }

    /// 通过 wecom-cli 上传
    fn upload_via_cli(&self, file_path: &Path) -> Result<WedriveResult> {
        let cli = self.wecom_cli_path.as_deref().unwrap_or("wecom-cli");
        let output = Command::new(cli)
            .args(["disk", "files", "upload", "--file-path"])
            .arg(file_path)
            .output()
            .context("执行 wecom-cli 上传失败")?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            return Ok(WedriveResult {
                success: false,
                file_path: None,
                file_id: None,
                error: Some(format!("{}\n{}", stdout, stderr)),
            });
        }

        // 解析 JSON 输出
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&stdout) {
            let file_path = v
                .get("file")
                .and_then(|f| f.get("path"))
                .and_then(|p| p.as_str())
                .map(|s| s.to_string());
            let file_id = v
                .get("file")
                .and_then(|f| f.get("id"))
                .and_then(|id| id.as_str())
                .map(|s| s.to_string());

            return Ok(WedriveResult {
                success: true,
                file_path,
                file_id,
                error: None,
            });
        }

        // 非 JSON 输出，尝试从文本提取
        Ok(WedriveResult {
            success: true,
            file_path: Some(stdout.trim().to_string()),
            file_id: None,
            error: None,
        })
    }

    /// 通过企业微信 API 直接上传（需要 access_token）
    fn upload_via_api(&self, file_path: &Path) -> Result<WedriveResult> {
        let corpid = self.corpid.as_ref().unwrap();
        let corpsecret = self.corpsecret.as_ref().unwrap();

        // 1. 获取 access_token
        let token_url = format!(
            "https://qyapi.weixin.qq.com/cgi-bin/gettoken?corpid={}&corpsecret={}",
            corpid, corpsecret
        );
        let token_resp = reqwest::blocking::get(&token_url)
            .context("获取 access_token 失败")?;
        let token_json: serde_json::Value = token_resp.json()?;
        let access_token = token_json
            .get("access_token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| anyhow!("获取 access_token 失败: {}", token_json))?;

        // 2. 上传文件到微盘
        // 企业微信微盘上传 API: https://qyapi.weixin.qq.com/cgi-bin/wedrive/file_upload
        let upload_url = format!(
            "https://qyapi.weixin.qq.com/cgi-bin/wedrive/file_upload?access_token={}",
            access_token
        );

        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");

        let form = reqwest::blocking::multipart::Form::new()
            .file("media", file_path)
            .with_context(|| format!("构建上传表单失败: {}", file_path.display()))?;

        let client = reqwest::blocking::Client::new();
        let resp = client
            .post(&upload_url)
            .multipart(form)
            .send()
            .context("微盘文件上传失败")?;

        let text = resp.text().unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&text)
            .with_context(|| format!("解析微盘响应失败: {}", text))?;

        let errcode = v.get("errcode").and_then(|x| x.as_i64()).unwrap_or(-1);
        if errcode != 0 {
            let errmsg = v.get("errmsg").and_then(|x| x.as_str()).unwrap_or("unknown");
            return Ok(WedriveResult {
                success: false,
                file_path: None,
                file_id: None,
                error: Some(format!("errcode={}: {}", errcode, errmsg)),
            });
        }

        let file_id = v
            .get("fileid")
            .or_else(|| v.get("file_id"))
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());

        Ok(WedriveResult {
            success: true,
            file_path: Some(file_name.to_string()),
            file_id,
            error: None,
        })
    }
}

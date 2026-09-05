use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use serde_json::json;
use std::path::Path;

/// 企业微信通知客户端
pub struct WechatNotifier {
    webhook_url: String,
    client: Client,
}

impl WechatNotifier {
    pub fn new(webhook_url: &str) -> Self {
        Self {
            webhook_url: webhook_url.to_string(),
            client: Client::new(),
        }
    }

    /// 发送文本消息
    pub fn send_text(&self, content: &str) -> Result<()> {
        let body = json!({
            "msgtype": "text",
            "text": {
                "content": content
            }
        });
        self.post(&body)
    }

    /// 发送 Markdown 消息
    pub fn send_markdown(&self, content: &str) -> Result<()> {
        let body = json!({
            "msgtype": "markdown",
            "markdown": {
                "content": content
            }
        });
        self.post(&body)
    }

    /// 发送文件消息（先上传获取 media_id，再发送）
    pub fn send_file(&self, file_path: &Path) -> Result<()> {
        let media_id = self.upload_file(file_path)?;
        let body = json!({
            "msgtype": "file",
            "file": {
                "media_id": media_id
            }
        });
        self.post(&body)
    }

    /// 上传文件到企业微信，返回 media_id
    fn upload_file(&self, file_path: &Path) -> Result<String> {
        if !file_path.exists() {
            return Err(anyhow!("文件不存在: {}", file_path.display()));
        }

        let _file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        let _file_bytes = std::fs::read(file_path)
            .with_context(|| format!("读取文件失败: {}", file_path.display()))?;

        // 构建上传 URL（从 webhook URL 中提取 key）
        let upload_url = self.webhook_url.replace(
            "/cgi-bin/webhook/send?",
            "/cgi-bin/webhook/upload_media?type=file&",
        );

        let form = reqwest::blocking::multipart::Form::new()
            .file("media", file_path)
            .with_context(|| format!("构建上传表单失败: {}", file_path.display()))?;

        let resp = self
            .client
            .post(&upload_url)
            .multipart(form)
            .send()
            .with_context(|| "上传文件到企业微信失败")?;

        let status = resp.status();
        let text = resp.text().unwrap_or_default();

        if !status.is_success() {
            return Err(anyhow!("上传文件失败 (HTTP {}): {}", status, text));
        }

        let v: serde_json::Value = serde_json::from_str(&text)
            .with_context(|| format!("解析上传响应失败: {}", text))?;

        let errcode = v.get("errcode").and_then(|x| x.as_i64()).unwrap_or(-1);
        if errcode != 0 {
            let errmsg = v.get("errmsg").and_then(|x| x.as_str()).unwrap_or("unknown");
            return Err(anyhow!("上传文件失败 (errcode {}): {}", errcode, errmsg));
        }

        let media_id = v
            .get("media_id")
            .and_then(|x| x.as_str())
            .ok_or_else(|| anyhow!("响应中无 media_id: {}", text))?
            .to_string();

        Ok(media_id)
    }

    fn post(&self, body: &serde_json::Value) -> Result<()> {
        let resp = self
            .client
            .post(&self.webhook_url)
            .json(body)
            .send()
            .with_context(|| "发送企业微信消息失败")?;

        let status = resp.status();
        let text = resp.text().unwrap_or_default();

        if !status.is_success() {
            return Err(anyhow!("企业微信请求失败 (HTTP {}): {}", status, text));
        }

        let v: serde_json::Value = serde_json::from_str(&text)
            .with_context(|| format!("解析企业微信响应失败: {}", text))?;

        let errcode = v.get("errcode").and_then(|x| x.as_i64()).unwrap_or(-1);
        if errcode != 0 {
            let errmsg = v.get("errmsg").and_then(|x| x.as_str()).unwrap_or("unknown");
            return Err(anyhow!("企业微信返回错误 (errcode {}): {}", errcode, errmsg));
        }

        Ok(())
    }
}

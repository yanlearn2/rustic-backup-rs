use anyhow::{anyhow, Context, Result};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 确保 rustic.exe 存在，不存在则自动下载
pub fn ensure_rustic(config_rustic_path: Option<&str>) -> Result<PathBuf> {
    if let Some(p) = config_rustic_path {
        let path = PathBuf::from(p);
        if path.exists() {
            return Ok(path);
        }
        anyhow::bail!("配置的 rustic 路径不存在: {}", path.display());
    }

    let bin_dir = default_bin_dir();
    let rustic_exe = bin_dir.join("rustic.exe");

    if rustic_exe.exists() {
        return Ok(rustic_exe);
    }

    println!("未找到 rustic.exe，开始自动下载...");
    download_rustic(&bin_dir, &rustic_exe)?;
    println!("✅ rustic.exe 下载完成");

    Ok(rustic_exe)
}

fn default_bin_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    exe_dir.join("bin")
}

fn download_rustic(bin_dir: &Path, target_exe: &Path) -> Result<()> {
    if !bin_dir.exists() {
        std::fs::create_dir_all(bin_dir)?;
    }

    // 1. 获取最新 release
    println!("  查询最新版本...");
    let client = reqwest::blocking::Client::builder()
        .user_agent("rustic-backup-rs")
        .build()?;

    let resp = client.get("https://api.github.com/repos/rustic-rs/rustic/releases/latest")
        .send()
        .with_context(|| "查询 GitHub release 失败")?;

    if !resp.status().is_success() {
        anyhow::bail!("查询 GitHub release 失败: HTTP {}", resp.status());
    }

    let release: serde_json::Value = resp.json()?;
    let tag = release.get("tag_name").and_then(|v| v.as_str()).unwrap_or("unknown");
    println!("  最新版本: {}", tag);

    // 2. 找 windows 下载链接（优先 zip，其次 tar.gz）
    let assets = release.get("assets").and_then(|v| v.as_array()).ok_or_else(|| anyhow!("release 无 assets"))?;

    let find_asset = |ext: &str| -> Option<(String, String)> {
        assets.iter().find_map(|a| {
            let name = a.get("name").and_then(|v| v.as_str())?;
            let url = a.get("browser_download_url").and_then(|v| v.as_str())?;
            let lower = name.to_lowercase();
            if lower.contains("windows") && lower.ends_with(ext) {
                Some((name.to_string(), url.to_string()))
            } else {
                None
            }
        })
    };

    let (asset_name, download_url) = find_asset(".zip")
        .or_else(|| find_asset(".tar.gz"))
        .ok_or_else(|| {
            let names: Vec<&str> = assets.iter()
                .filter_map(|a| a.get("name").and_then(|v| v.as_str()))
                .collect();
            anyhow!("未找到 windows 版本，可用资产: {:?}", names)
        })?;

    println!("  下载: {}", asset_name);

    // 3. 下载（优先用系统 curl.exe，稳定可靠带进度；fallback 到 reqwest 一次性下载）
    let bytes = if curl_available() {
        download_via_curl(&download_url, bin_dir)?
    } else {
        println!("  （未检测到 curl.exe，使用内置下载）");
        download_via_reqwest(&client, &download_url)?
    };

    println!("  下载完成: {} bytes", bytes.len());

    // 4. 解压
    let lower_name = asset_name.to_lowercase();
    if lower_name.ends_with(".zip") {
        extract_zip(&bytes, target_exe)?;
    } else if lower_name.ends_with(".tar.gz") {
        extract_tar_gz(&bytes, target_exe)?;
    } else {
        anyhow::bail!("不支持的压缩格式: {}", asset_name);
    }

    Ok(())
}

fn curl_available() -> bool {
    Command::new("curl.exe")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn download_via_curl(url: &str, bin_dir: &Path) -> Result<Vec<u8>> {
    let temp_file = bin_dir.join("rustic-download.tmp");
    let _ = std::fs::remove_file(&temp_file);

    let status = Command::new("curl.exe")
        .args(["-L", "-o"])
        .arg(&temp_file)
        .arg(url)
        .arg("--progress-bar")
        .arg("--retry")
        .arg("3")
        .arg("--retry-delay")
        .arg("2")
        .status()
        .with_context(|| "执行 curl.exe 失败")?;

    if !status.success() {
        anyhow::bail!("curl 下载失败 (exit {:?})", status.code());
    }

    let bytes = std::fs::read(&temp_file)
        .with_context(|| "读取下载文件失败")?;
    let _ = std::fs::remove_file(&temp_file);

    Ok(bytes)
}

fn download_via_reqwest(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>> {
    let resp = client.get(url).send()?;
    if !resp.status().is_success() {
        anyhow::bail!("下载失败: HTTP {}", resp.status());
    }
    let total = resp.content_length().unwrap_or(0);
    println!("  总大小: {}", fmt_size(total));
    let bytes = resp.bytes()?.to_vec();
    Ok(bytes)
}

fn fmt_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut idx = 0;
    while size >= 1024.0 && idx < UNITS.len() - 1 {
        size /= 1024.0;
        idx += 1;
    }
    format!("{:.1} {}", size, UNITS[idx])
}

fn extract_zip(bytes: &[u8], target_exe: &Path) -> Result<()> {
    let reader = Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(reader).with_context(|| "解析 zip 失败")?;

    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        let name = file.name().to_string();
        if name.to_lowercase().ends_with("rustic.exe") || name == "rustic.exe" {
            if let Some(parent) = target_exe.parent() {
                if !parent.exists() { std::fs::create_dir_all(parent)?; }
            }
            let mut out = std::fs::File::create(target_exe)?;
            std::io::copy(&mut file, &mut out)?;
            println!("  解压: {} → {}", name, target_exe.display());
            return Ok(());
        }
    }
    anyhow::bail!("zip 中未找到 rustic.exe")
}

fn extract_tar_gz(bytes: &[u8], target_exe: &Path) -> Result<()> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let tar = GzDecoder::new(bytes);
    let mut archive = Archive::new(tar);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let name = path.to_string_lossy().to_string();
        if name.to_lowercase().ends_with("rustic.exe") || name == "rustic.exe" {
            if let Some(parent) = target_exe.parent() {
                if !parent.exists() { std::fs::create_dir_all(parent)?; }
            }
            let mut out = std::fs::File::create(target_exe)?;
            std::io::copy(&mut entry, &mut out)?;
            println!("  解压: {} → {}", name, target_exe.display());
            return Ok(());
        }
    }
    anyhow::bail!("tar.gz 中未找到 rustic.exe")
}

pub fn rustic_exists(config_rustic_path: Option<&str>) -> bool {
    if let Some(p) = config_rustic_path {
        return Path::new(p).exists();
    }
    default_bin_dir().join("rustic.exe").exists()
}

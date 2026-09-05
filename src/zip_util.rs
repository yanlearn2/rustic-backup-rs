use anyhow::{Context, Result};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use zip::write::FileOptions;
use zip::CompressionMethod;

/// 将多个文件打包成 zip
/// files: 要打包的文件路径列表
/// output: 输出 zip 路径
/// base_dir: 用于计算 zip 内相对路径的基准目录（可选）
pub fn zip_files(files: &[PathBuf], output: &Path, base_dir: Option<&Path>) -> Result<()> {
    if let Some(parent) = output.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let file = File::create(output)
        .with_context(|| format!("创建 zip 文件失败: {}", output.display()))?;
    let mut zip = zip::ZipWriter::new(file);

    let options: FileOptions<'_, ()> = FileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);

    for file_path in files {
        if !file_path.exists() {
            eprintln!("警告: 文件不存在，跳过: {}", file_path.display());
            continue;
        }

        // 计算 zip 内的文件名
        let name_in_zip = if let Some(base) = base_dir {
            file_path
                .strip_prefix(base)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string()
        } else {
            file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string()
        };

        zip.start_file(&name_in_zip, options)?;

        let f = File::open(file_path)
            .with_context(|| format!("打开文件失败: {}", file_path.display()))?;
        let mut reader = BufReader::new(f);
        std::io::copy(&mut reader, &mut zip)?;
    }

    zip.finish()?;
    Ok(())
}

/// 扫描目录下匹配 glob 模式的文件（仅根目录，不递归）
pub fn scan_root_files(dir: &Path, patterns: &[String], recursive: bool) -> Result<Vec<PathBuf>> {
    use globset::{Glob, GlobSetBuilder};

    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        builder.add(Glob::new(p)?);
    }
    let set = builder.build()?;

    let mut results = Vec::new();

    if recursive {
        // 递归扫描整个目录树
        for entry in walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if set.is_match(name) {
                    results.push(path.to_path_buf());
                }
            }
        }
    } else {
        // 只扫描根目录
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if set.is_match(name) {
                        results.push(path);
                    }
                }
            }
        }
    }

    Ok(results)
}

/// 应用正则排除过滤
pub fn filter_by_regex(files: Vec<PathBuf>, regex_patterns: &[String]) -> Result<Vec<PathBuf>> {
    if regex_patterns.is_empty() {
        return Ok(files);
    }

    let regexes: Vec<regex::Regex> = regex_patterns
        .iter()
        .map(|p| regex::Regex::new(p))
        .collect::<Result<Vec<_>, _>>()?;

    let filtered = files
        .into_iter()
        .filter(|f| {
            let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let path_str = f.to_string_lossy();
            !regexes.iter().any(|r| r.is_match(name) || r.is_match(&path_str))
        })
        .collect();

    Ok(filtered)
}

/// 复制文件到临时目录（解决文件占用问题）
pub fn copy_files_to_temp(files: &[PathBuf], temp_dir: &Path) -> Result<Vec<PathBuf>> {
    if !temp_dir.exists() {
        std::fs::create_dir_all(temp_dir)?;
    }

    let mut copied = Vec::new();
    for src in files {
        if let Some(name) = src.file_name() {
            let dst = temp_dir.join(name);
            match std::fs::copy(src, &dst) {
                Ok(_) => copied.push(dst),
                Err(e) => eprintln!("警告: 复制文件失败（可能被占用）{}: {}", src.display(), e),
            }
        }
    }
    Ok(copied)
}

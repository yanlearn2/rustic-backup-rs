# rustic-backup-rs

基于 Rust 编写的 rustic 备份管理工具。配置驱动，单二进制，企业微信通知。

## 快速开始

```powershell
# 1. 生成配置文件
.\rustic-backup.exe gen-config

# 2. 编辑 config.json，填写仓库密码、源路径、webhook

# 3. 初始化仓库
.\rustic-backup.exe init

# 4. 注册计划任务（每天 07:30 自动执行）
.\rustic-backup.exe install --time 07:30
```

## 命令一览

| 命令 | 说明 |
|------|------|
| `gen-config` | 生成默认配置文件 |
| `init` | 初始化备份仓库 |
| `install --time 07:30` | 注册 Windows 计划任务 |
| `uninstall` | 移除计划任务 |
| `run` | 手动执行所有到期备份 |
| `run --task 1` | 只执行指定任务 |
| `tasks` | 查看所有任务状态 |
| `tasks --enable 1` / `--disable 1` | 启用/禁用任务 |
| `status` | 查看备份状态 + 仓库信息 |
| `list` | 列出快照 |
| `list --task 1 --last 7` | 过滤快照 |
| `restore <ID> --target D:\restore` | 恢复快照 |
| `logs` | 查看今日日志 |
| `prune` | 清理旧快照（默认预览，加 `--yes` 执行） |

## 配置说明

### 任务配置字段

| 字段 | 说明 |
|------|------|
| `name` | 任务名称（显示用，也作为标签） |
| `source` | 源目录路径 |
| `schedule` | `daily` / `weekly` / `monthly` / `yearly` |
| `time` | 执行时间 HH:MM |
| `weekday` | 每周几执行（0=周日，仅 weekly） |
| `monthday` | 每月几号执行（仅 monthly） |
| `include` | glob 包含模式（rustic 原生） |
| `exclude` | glob 排除模式（rustic 原生） |
| `exclude_regex` | 正则排除（应用层过滤） |
| `zip_root_files` | 是否打包根目录文件发企业微信 |
| `root_file_patterns` | 根目录文件匹配模式（默认 *.xlsx, *.xls） |
| `notify.skip_first_run` | 首次备份跳过文件发送 |
| `notify.incremental` | 增量模式，只打包变化文件 |
| `enabled` | 是否启用 |

### 保留策略

```json
"retention": {
  "daily": 15,
  "weekly": 12,
  "monthly": 12,
  "yearly": 3
}
```

## 目录结构

```
rustic-backup/
├── rustic-backup.exe   # 主程序
├── config.json          # 配置文件
├── bin/rustic.exe       # rustic 引擎（需自行放置）
├── repo/                # 备份仓库（自动创建）
├── logs/                # 日志（自动创建）
└── temp/                # 临时文件（自动创建）
```

## 注意事项

1. **仓库密码**是解密唯一密钥，务必单独备份保存
2. **rustic.exe** 需放在 `bin/` 目录下，或在 config.json 中指定 `rustic_path`
3. **NAS 权限**：确保运行任务的用户有访问源目录的权限
4. **微盘上传**：需要系统安装 wecom-cli，或在配置中指定 corpid/corpsecret

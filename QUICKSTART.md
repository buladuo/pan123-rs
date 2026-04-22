# pan123-rs 快速使用指南

## 安装

### Linux / macOS

```bash
# 克隆项目
git clone <repository-url>
cd pan123-rs

# 运行构建脚本
./scripts/build.sh

# 安装
cd dist/pan123-*-linux-*/  # 或 darwin
./install.sh
```

### Windows

```powershell
# 克隆项目
git clone <repository-url>
cd pan123-rs

# 运行构建脚本
.\scripts\build.ps1

# 安装
cd dist\pan123-*-windows-*\
# 右键点击 install.bat -> 以管理员身份运行
```

## 基本使用

### 1. 登录

```bash
pan123 login
# 使用 123 盘 App 或微信扫描二维码
```

### 2. 浏览文件

```bash
# 列出文件
pan123 ls

# 切换目录
pan123 cd Documents

# 显示目录树
pan123 tree
```

### 3. 上传文件

```bash
# 上传单个文件
pan123 upload file.txt

# 上传目录（并发上传）
pan123 upload my-folder --jobs 4

# 覆盖已存在的文件
pan123 upload file.txt --duplicate overwrite
```

### 4. 下载文件

```bash
# 下载文件
pan123 download file.txt

# 下载到指定目录
pan123 download file.txt --dir ~/Downloads

# 下载多个文件
pan123 download file1.txt file2.txt folder-name
```

### 5. 文件管理

```bash
# 创建文件夹
pan123 mkdir NewFolder

# 重命名
pan123 rename old.txt new.txt

# 移动文件
pan123 mv TargetFolder file1.txt file2.txt

# 复制文件
pan123 cp BackupFolder important.txt

# 删除文件
pan123 rm unwanted.txt
```

### 6. 交互式 Shell

```bash
pan123 shell

# 在 Shell 中使用所有命令
pan123> ls
pan123> cd Documents
pan123> upload file.txt
pan123> exit
```

## 常用场景

### 批量上传

```bash
# 上传整个项目目录
pan123 upload ~/projects/my-app --jobs 8

# 上传多个文件
for file in *.pdf; do
    pan123 upload "$file"
done
```

### 自动备份

```bash
#!/bin/bash
# backup.sh

pan123 cd Backup || pan123 mkdir Backup
pan123 cd Backup
pan123 upload ~/important-data --duplicate overwrite
```

### 批量下载

```bash
# 下载当前目录所有文件
pan123 download $(pan123 ls | awk '{print $NF}')
```

## 配置

### Token 存储位置

- **Linux**: Secret Service (gnome-keyring)
- **macOS**: Keychain
- **Windows**: Credential Manager

### 配置文件

- **Linux/macOS**: `~/.config/pan123/`
- **Windows**: `%APPDATA%\pan123\`

## 故障排除

### Token 失效

```bash
pan123 status  # 检查状态
pan123 login   # 重新登录
```

### 上传失败

```bash
# 增加重试次数
pan123 upload file.txt --retries 10

# 减少并发数
pan123 upload folder --jobs 2
```

### 下载中断

```bash
# 自动断点续传，重新运行即可
pan123 download large-file.zip
```

## 更多帮助

```bash
# 查看所有命令
pan123 --help

# 查看特定命令帮助
pan123 upload --help
pan123 download --help
```

## 完整文档

查看 [README.md](README.md) 获取完整文档。

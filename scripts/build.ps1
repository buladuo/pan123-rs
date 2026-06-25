# pan123-rs Windows 构建脚本

$ErrorActionPreference = "Stop"

# 项目信息
$ProjectName = "pan123"
$Version = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches.Groups[1].Value

Write-Host "========================================" -ForegroundColor Blue
Write-Host "  pan123-rs 构建脚本 (Windows)" -ForegroundColor Blue
Write-Host "  版本: $Version" -ForegroundColor Blue
Write-Host "========================================" -ForegroundColor Blue
Write-Host ""

# 检查 Rust 环境
Write-Host "[1/5] 检查 Rust 环境..." -ForegroundColor Yellow
try {
    $RustVersion = cargo --version
    Write-Host "✓ Rust 环境正常: $RustVersion" -ForegroundColor Green
} catch {
    Write-Host "错误: 未找到 cargo 命令" -ForegroundColor Red
    Write-Host "请先安装 Rust: https://rustup.rs/" -ForegroundColor Yellow
    exit 1
}
Write-Host ""

# 清理旧的构建
Write-Host "[2/5] 清理旧的构建文件..." -ForegroundColor Yellow
cargo clean
Write-Host "✓ 清理完成" -ForegroundColor Green
Write-Host ""

# 编译 Release 版本
Write-Host "[3/5] 编译 Release 版本..." -ForegroundColor Yellow
Write-Host "这可能需要几分钟时间，请耐心等待..." -ForegroundColor Blue
cargo build --release
if ($LASTEXITCODE -ne 0) {
    Write-Host "编译失败" -ForegroundColor Red
    exit 1
}
Write-Host "✓ 编译完成" -ForegroundColor Green
Write-Host ""

# 检查编译产物
Write-Host "[4/5] 检查编译产物..." -ForegroundColor Yellow
$BinaryPath = "target\release\$ProjectName.exe"
if (-not (Test-Path $BinaryPath)) {
    Write-Host "错误: 未找到编译产物 $BinaryPath" -ForegroundColor Red
    exit 1
}

$BinarySize = (Get-Item $BinaryPath).Length / 1MB
$BinarySizeStr = "{0:N2} MB" -f $BinarySize
Write-Host "✓ 二进制文件: $BinaryPath ($BinarySizeStr)" -ForegroundColor Green
Write-Host ""

# 创建发布包
Write-Host "[5/5] 创建发布包..." -ForegroundColor Yellow

# 检测架构
$Arch = $env:PROCESSOR_ARCHITECTURE.ToLower()
if ($Arch -eq "amd64") {
    $Arch = "x86_64"
}

$ReleaseName = "$ProjectName-$Version-windows-$Arch"
$ReleaseDir = "dist\$ReleaseName"

# 创建发布目录
New-Item -ItemType Directory -Force -Path $ReleaseDir | Out-Null

# 复制文件
Copy-Item $BinaryPath "$ReleaseDir\$ProjectName.exe"
if (Test-Path "README.md") {
    Copy-Item "README.md" "$ReleaseDir\"
}
if (Test-Path "LICENSE") {
    Copy-Item "LICENSE" "$ReleaseDir\"
}

# 创建安装脚本
$InstallScript = @'
@echo off
setlocal enabledelayedexpansion

set BINARY_NAME=pan123.exe
set INSTALL_DIR=%ProgramFiles%\pan123

echo ========================================
echo   pan123-rs 安装程序
echo ========================================
echo.

:: 检查管理员权限
net session >nul 2>&1
if errorlevel 1 (
    echo 错误: 需要管理员权限
    echo 请右键点击此脚本，选择"以管理员身份运行"
    echo.
    pause
    exit /b 1
)

:: 检查是否已安装
if exist "%INSTALL_DIR%\%BINARY_NAME%" (
    echo 检测到已安装的版本

    :: 获取当前版本
    "%INSTALL_DIR%\%BINARY_NAME%" --version >nul 2>&1
    if not errorlevel 1 (
        for /f "tokens=*" %%i in ('"%INSTALL_DIR%\%BINARY_NAME%" --version 2^>nul') do set CURRENT_VERSION=%%i
        echo 当前版本: !CURRENT_VERSION!
    )

    echo.
    echo 将卸载旧版本并安装新版本
    echo.

    :: 尝试停止正在运行的进程
    tasklist | find /i "%BINARY_NAME%" >nul
    if not errorlevel 1 (
        echo 检测到 %BINARY_NAME% 正在运行，尝试结束进程...
        taskkill /F /IM "%BINARY_NAME%" >nul 2>&1
        timeout /t 2 >nul
    )

    :: 删除旧版本
    echo 正在删除旧版本...
    del /F /Q "%INSTALL_DIR%\%BINARY_NAME%" >nul 2>&1
    if exist "%INSTALL_DIR%\%BINARY_NAME%" (
        echo 警告: 无法删除旧版本，文件可能被占用
        echo 请关闭所有 pan123 进程后重试
        pause
        exit /b 1
    )
    echo ✓ 旧版本已删除
    echo.
)

:: 创建安装目录
if not exist "%INSTALL_DIR%" (
    echo 创建安装目录: %INSTALL_DIR%
    mkdir "%INSTALL_DIR%"
)

:: 复制新文件
echo 正在安装 %BINARY_NAME%...
copy /Y "%BINARY_NAME%" "%INSTALL_DIR%\%BINARY_NAME%" >nul
if errorlevel 1 (
    echo 错误: 安装失败
    pause
    exit /b 1
)

:: 验证安装
if not exist "%INSTALL_DIR%\%BINARY_NAME%" (
    echo 错误: 安装验证失败
    pause
    exit /b 1
)

echo ✓ 文件已复制到 %INSTALL_DIR%

:: 添加到 PATH
set "PATH_TO_ADD=%INSTALL_DIR%"
for /f "skip=2 tokens=3*" %%a in ('reg query "HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment" /v Path 2^>nul') do set "CURRENT_PATH=%%a %%b"

echo %CURRENT_PATH% | find /i "%PATH_TO_ADD%" >nul
if errorlevel 1 (
    echo 正在添加到系统 PATH...
    setx /M PATH "%CURRENT_PATH%;%PATH_TO_ADD%" >nul
    if errorlevel 1 (
        echo 警告: 无法自动添加到 PATH
        echo 请手动添加到系统环境变量: %PATH_TO_ADD%
    ) else (
        echo ✓ 已添加到系统 PATH
    )
) else (
    echo ✓ 已存在于系统 PATH
)

:: 获取新版本
echo.
for /f "tokens=*" %%i in ('"%INSTALL_DIR%\%BINARY_NAME%" --version 2^>nul') do set NEW_VERSION=%%i

echo.
echo ========================================
echo ✓ 安装完成！
echo ========================================
echo.
echo 安装版本: !NEW_VERSION!
echo 安装位置: %INSTALL_DIR%
echo.
echo 请重新打开命令提示符或 PowerShell，然后运行:
echo   pan123 --version    查看版本
echo   pan123 --help       查看帮助信息
echo   pan123 login        登录账号
echo   pan123 shell        启动交互式 Shell
echo.
echo 卸载方法: 运行 uninstall.bat
echo.
pause
'@

Set-Content -Path "$ReleaseDir\install.bat" -Value $InstallScript -Encoding ASCII

# 创建卸载脚本
$UninstallScript = @'
@echo off
setlocal enabledelayedexpansion

set BINARY_NAME=pan123.exe
set INSTALL_DIR=%ProgramFiles%\pan123

echo ========================================
echo   pan123-rs 卸载程序
echo ========================================
echo.

:: 检查管理员权限
net session >nul 2>&1
if errorlevel 1 (
    echo 错误: 需要管理员权限
    echo 请右键点击此脚本，选择"以管理员身份运行"
    echo.
    pause
    exit /b 1
)

:: 检查是否已安装
if not exist "%INSTALL_DIR%\%BINARY_NAME%" (
    echo 未找到已安装的 %BINARY_NAME%
    echo 安装位置: %INSTALL_DIR%
    echo.
    pause
    exit /b 0
)

:: 获取当前版本
for /f "tokens=*" %%i in ('"%INSTALL_DIR%\%BINARY_NAME%" --version 2^>nul') do set CURRENT_VERSION=%%i
echo 检测到安装版本: !CURRENT_VERSION!
echo 安装位置: %INSTALL_DIR%
echo.

set /p CONFIRM="确认卸载? (Y/N): "
if /i not "%CONFIRM%"=="Y" (
    echo 已取消卸载
    pause
    exit /b 0
)

echo.
echo 正在卸载...

:: 尝试停止正在运行的进程
tasklist | find /i "%BINARY_NAME%" >nul
if not errorlevel 1 (
    echo 检测到 %BINARY_NAME% 正在运行，尝试结束进程...
    taskkill /F /IM "%BINARY_NAME%" >nul 2>&1
    timeout /t 2 >nul
)

:: 删除文件
del /F /Q "%INSTALL_DIR%\%BINARY_NAME%" >nul 2>&1
if errorlevel 1 (
    echo 错误: 无法删除文件，可能被占用
    echo 请关闭所有 pan123 进程后重试
    pause
    exit /b 1
)

if exist "%INSTALL_DIR%\%BINARY_NAME%" (
    echo 错误: 删除失败
    pause
    exit /b 1
)

echo ✓ 已删除程序文件

:: 删除目录（如果为空）
rmdir "%INSTALL_DIR%" 2>nul
if not exist "%INSTALL_DIR%" (
    echo ✓ 已删除安装目录
)

echo.
echo ========================================
echo ✓ 卸载完成！
echo ========================================
echo.
echo 注意:
echo 1. 配置文件保留在: %%APPDATA%%\pan123\
echo 2. PATH 环境变量未自动清理
echo    如需清理请手动从系统 PATH 中删除: %INSTALL_DIR%
echo 3. Windows 凭据管理器中的登录信息未删除
echo    如需删除请在"控制面板 ^> 凭据管理器"中手动删除
echo.
pause
'@

Set-Content -Path "$ReleaseDir\uninstall.bat" -Value $UninstallScript -Encoding ASCII

# 创建 README
$ReadmeContent = @"
# pan123-rs for Windows

版本: $Version

## 安装方法

### 方法 1: 使用安装脚本（推荐）

1. 右键点击 install.bat
2. 选择"以管理员身份运行"
3. 按照提示完成安装
4. 重新打开命令提示符
5. 运行 ``pan123 --version`` 验证安装

### 方法 2: 手动安装

1. 将 pan123.exe 复制到任意目录，例如: C:\Program Files\pan123\
2. 将该目录添加到系统 PATH 环境变量:
   - 右键"此电脑" -> 属性 -> 高级系统设置
   - 环境变量 -> 系统变量 -> Path -> 编辑
   - 新建 -> 输入目录路径 -> 确定
3. 重新打开命令提示符
4. 运行 ``pan123 --version`` 验证安装

### 方法 3: 便携模式

直接运行 pan123.exe，无需安装。

## 快速开始

``````
# 登录
pan123 login

# 查看帮助
pan123 --help

# 启动交互式 Shell
pan123 shell
``````

## 卸载

运行 uninstall.bat（需要管理员权限）

## 配置文件位置

- Token: Windows 凭据管理器
- 配置: %APPDATA%\pan123\

## 更多信息

查看 README.md 获取完整文档。
"@

Set-Content -Path "$ReleaseDir\INSTALL.txt" -Value $ReadmeContent -Encoding UTF8

# 打包为 ZIP
Write-Host "正在创建 ZIP 压缩包..." -ForegroundColor Blue
$ZipPath = "dist\$ReleaseName.zip"
if (Test-Path $ZipPath) {
    Remove-Item $ZipPath -Force
}

# 使用 .NET 压缩
Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::CreateFromDirectory($ReleaseDir, $ZipPath)

$PackageSize = (Get-Item $ZipPath).Length / 1MB
$PackageSizeStr = "{0:N2} MB" -f $PackageSize
Write-Host "✓ 发布包已创建: $ZipPath ($PackageSizeStr)" -ForegroundColor Green
Write-Host ""

# 显示安装说明
Write-Host "========================================" -ForegroundColor Blue
Write-Host "构建成功！" -ForegroundColor Green
Write-Host "========================================" -ForegroundColor Blue
Write-Host ""
Write-Host "安装方法 1 - 使用安装脚本:" -ForegroundColor Yellow
Write-Host "  cd dist\$ReleaseName"
Write-Host "  右键点击 install.bat -> 以管理员身份运行"
Write-Host ""
Write-Host "安装方法 2 - 手动安装:" -ForegroundColor Yellow
Write-Host "  1. 复制 target\release\$ProjectName.exe 到 C:\Program Files\pan123\"
Write-Host "  2. 将 C:\Program Files\pan123\ 添加到系统 PATH"
Write-Host ""
Write-Host "安装方法 3 - 解压发布包:" -ForegroundColor Yellow
Write-Host "  1. 解压 dist\$ReleaseName.zip"
Write-Host "  2. 右键点击 install.bat -> 以管理员身份运行"
Write-Host ""
Write-Host "验证安装:" -ForegroundColor Yellow
Write-Host "  $ProjectName --version"
Write-Host ""
Write-Host "开始使用:" -ForegroundColor Yellow
Write-Host "  $ProjectName login"
Write-Host "  $ProjectName shell"
Write-Host ""

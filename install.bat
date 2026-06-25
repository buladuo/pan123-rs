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

:: 查找二进制文件
set BINARY_PATH=
if exist "target\release\%BINARY_NAME%" (
    set BINARY_PATH=target\release\%BINARY_NAME%
) else if exist "%BINARY_NAME%" (
    set BINARY_PATH=%BINARY_NAME%
) else (
    echo 错误: 未找到 %BINARY_NAME%
    echo.
    echo 请先编译项目:
    echo   cargo build --release
    echo.
    echo 或运行构建脚本:
    echo   powershell -ExecutionPolicy Bypass -File scripts\build.ps1
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
copy /Y "%BINARY_PATH%" "%INSTALL_DIR%\%BINARY_NAME%" >nul
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

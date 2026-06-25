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

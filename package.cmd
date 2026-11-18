@echo off
REM ============================================================
REM  打包发布：MSI + 便携版 exe
REM
REM  用法：
REM    package.cmd            打包
REM    package.cmd clean      清理后重新打包
REM
REM  产物（在 dist-release\ 下）：
REM    SkillMarket-<版本>-x64.msi        安装包
REM    SkillMarket-<版本>-Portable.zip   便携版
REM ============================================================

setlocal enabledelayedexpansion
cd /d "%~dp0"

if /I "%~1"=="clean" (
  echo [clean] 清理构建产物...
  if exist "dist-release" rmdir /s /q "dist-release"
)

REM --- 前置检查 ---
where pnpm >nul 2>&1 || (echo [ERROR] 未找到 pnpm & exit /b 1)
where cargo >nul 2>&1 || (echo [ERROR] 未找到 cargo & exit /b 1)

REM --- WiX 预检 ---
REM 首次打包时 Tauri 会从 GitHub 下载 WiX 工具链（约 5MB），
REM 国内网络常卡住甚至失败（报 "Peer disconnected"）。
REM 缓存位置：%LOCALAPPDATA%\tauri\WixTools314
set WIX_CACHE=%LOCALAPPDATA%\tauri\WixTools314
if not exist "%WIX_CACHE%" (
  echo.
  echo [WARN] 未找到 WiX 工具链缓存 ^(%WIX_CACHE%^)
  echo.
  echo        首次打包需要从 GitHub 下载，国内网络可能失败并报
  echo        "Peer disconnected"。若本次构建卡住，可手动预置：
  echo.
  echo        1. 下载 https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip
  echo        2. 解压到 %WIX_CACHE%
  echo        3. 重新运行本脚本
  echo.
  pause
)

REM --- 构建前端 ---
echo [1/3] 构建前端...
call pnpm build:renderer
if errorlevel 1 (echo [ERROR] 前端构建失败 & exit /b 1)

REM --- 打包（MSI + 便携版）---
echo [2/3] 打包 MSI...
call pnpm tauri build --bundles msi
if errorlevel 1 (echo [ERROR] MSI 打包失败 & exit /b 1)

REM --- 收集产物 ---
echo [3/3] 收集产物...
if not exist "dist-release" mkdir "dist-release"

REM CARGO_TARGET_DIR 可能被外部设置（如本机为 C:\ck 以规避路径过长），
REM 因此产物目录要跟着变，不能写死 src-tauri\target。
if defined CARGO_TARGET_DIR (
  set TARGET_ROOT=%CARGO_TARGET_DIR%
  echo   [info] 使用 CARGO_TARGET_DIR=%CARGO_TARGET_DIR%
) else (
  set TARGET_ROOT=%~dp0src-tauri\target
)

set MSI_SRC=%TARGET_ROOT%\release\bundle\msi
set EXE_SRC=%TARGET_ROOT%\release\skill-market.exe

if exist "%MSI_SRC%" (
  for %%f in ("%MSI_SRC%\*.msi") do (
    copy /y "%%f" "%~dp0dist-release\" >nul 2>&1
    echo   MSI: %%~nxf
  )
) else (
  echo   [WARN] 未找到 MSI 源目录: %MSI_SRC%
)

if exist "%EXE_SRC%" (
  if exist "%~dp0dist-release\portable" rmdir /s /q "%~dp0dist-release\portable"
  mkdir "%~dp0dist-release\portable"
  copy /y "%EXE_SRC%" "%~dp0dist-release\portable\" >nul
  echo portable=true> "%~dp0dist-release\portable\portable.ini"
  powershell -NoProfile -Command "Compress-Archive -Path '%~dp0dist-release\portable\*' -DestinationPath '%~dp0dist-release\SkillMarket-Portable.zip' -Force"
  rmdir /s /q "%~dp0dist-release\portable"
  echo   Portable: SkillMarket-Portable.zip
) else (
  echo   [WARN] 未找到 exe: %EXE_SRC%
)

echo.
echo 完成。产物在 dist-release\ 下：
dir /b "dist-release"
echo.
echo 提示：目标机器需已安装 WebView2 Runtime。
echo       Windows 11 自带；Win10 多数已由微软推送。
echo       缺失时程序会弹提示，需先装内网共享盘的离线包。

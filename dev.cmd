@echo off
REM ============================================================
REM  Skill Market 开发环境启动脚本
REM
REM  解决一个反复出现的问题：上次异常退出会残留 vite 进程占住
REM  3100 端口，导致下次启动时前端起不来、应用窗口变成空壳
REM  （只有 15x15 的辅助窗口，没有主窗口）。
REM
REM  用法: dev.cmd
REM ============================================================

cd /d "%~dp0"

echo [1/3] 清理上次残留的进程...
taskkill /F /IM skill-market.exe >nul 2>&1

REM 找出占用 3100 的进程并结束
for /f "tokens=5" %%a in ('netstat -ano ^| findstr ":3100" ^| findstr "LISTENING"') do (
  echo   结束占用 3100 端口的进程 PID=%%a
  taskkill /F /PID %%a >nul 2>&1
)

timeout /t 1 /nobreak >nul

echo [2/3] 检查端口...
netstat -ano | findstr ":3100" | findstr "LISTENING" >nul 2>&1
if %errorlevel%==0 (
  echo   [ERROR] 端口 3100 仍被占用，请手动处理后重试
  exit /b 1
)
echo   端口空闲

echo [3/3] 启动...
set CARGO_TARGET_DIR=C:\ck
call pnpm tauri dev

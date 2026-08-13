@echo off
setlocal

cd /d "%~dp0"
title CADEgg Dev Launcher
set "LOG=%~dp0cadegg-launch.log"

echo ==== CADEgg launcher %DATE% %TIME% ==== > "%LOG%"
echo Script: %~f0 >> "%LOG%"
echo Working directory: %CD% >> "%LOG%"
echo User profile: %USERPROFILE% >> "%LOG%"
echo. >> "%LOG%"

if exist "%ProgramFiles%\nodejs\npm.cmd" set "PATH=%ProgramFiles%\nodejs;%PATH%"
if exist "%USERPROFILE%\.cargo\bin\cargo.exe" set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

echo ========================================
echo  CADEgg Dev Launcher
echo ========================================
echo Working directory: %CD%
echo Log file: %LOG%
echo.

echo [STEP] Checking npm.cmd... >> "%LOG%"
where npm.cmd >> "%LOG%" 2>&1
if errorlevel 1 (
  echo [ERROR] npm.cmd was not found. Please install Node.js first.
  echo Download: https://nodejs.org/
  echo [ERROR] npm.cmd was not found. >> "%LOG%"
  echo.
  pause
  exit /b 1
)

echo [STEP] Checking cargo.exe... >> "%LOG%"
where cargo.exe >> "%LOG%" 2>&1
if errorlevel 1 (
  if exist "%USERPROFILE%\.cargo\bin\cargo.exe" (
    set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
    echo [INFO] Added user Cargo bin to PATH. >> "%LOG%"
  ) else (
    echo [ERROR] cargo.exe was not found. CADEgg requires the Rust toolchain.
    echo Install Rust: https://rustup.rs/
    echo Then run this launcher again.
    echo [ERROR] cargo.exe was not found. >> "%LOG%"
    echo.
    pause
    exit /b 1
  )
)

echo [INFO] Node/NPM:
node --version
call npm.cmd --version
echo [INFO] Cargo:
cargo.exe --version
echo.

echo [INFO] Node/NPM: >> "%LOG%"
node --version >> "%LOG%" 2>&1
call npm.cmd --version >> "%LOG%" 2>&1
echo [INFO] Cargo: >> "%LOG%"
cargo.exe --version >> "%LOG%" 2>&1
echo. >> "%LOG%"

echo [STEP] Checking port 1420... >> "%LOG%"
netstat -ano | findstr ":1420 " >nul 2>nul
if not errorlevel 1 (
  echo [ERROR] Port 1420 is already in use. CADEgg or Vite may already be running.
  echo Close the old terminal window, or end the stale node.exe process in Task Manager.
  echo.
  netstat -ano | findstr ":1420 "
  echo [ERROR] Port 1420 is already in use. >> "%LOG%"
  netstat -ano | findstr ":1420 " >> "%LOG%" 2>&1
  pause
  exit /b 1
)

if not exist "node_modules" (
  echo [INFO] node_modules was not found. Installing frontend dependencies...
  echo [STEP] Running npm install... >> "%LOG%"
  powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "& npm.cmd install 2>&1 | Tee-Object -FilePath '%LOG%' -Append; exit $LASTEXITCODE"
  if errorlevel 1 (
    echo.
    echo [ERROR] npm install failed. See log: %LOG%
    pause
    exit /b 1
  )
  echo.
)

echo [INFO] Starting CADEgg...
echo First launch may need several minutes to compile Rust/Tauri.
echo Keep this window open. Closing it stops the dev server.
echo If launch fails, check the log file: %LOG%
echo.

echo [STEP] Running npm.cmd run tauri -- dev... >> "%LOG%"
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "& npm.cmd run tauri -- dev 2>&1 | Tee-Object -FilePath '%LOG%' -Append; exit $LASTEXITCODE"
echo [INFO] tauri dev exit code: %ERRORLEVEL% >> "%LOG%"

echo.
echo [INFO] CADEgg has exited.
pause

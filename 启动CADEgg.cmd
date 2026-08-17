@echo off
setlocal EnableExtensions

cd /d "%~dp0"
title CADEgg Launcher

set "ROOT=%CD%"
set "LOG=%ROOT%\cadegg-launch.log"
set "MODE=app"
if /I "%~1"=="--dev" set "MODE=dev"
if /I "%~1"=="dev" set "MODE=dev"

> "%LOG%" (
  echo ==== CADEgg launcher %DATE% %TIME% ====
  echo Script: %~f0
  echo Working directory: %ROOT%
  echo Mode: %MODE%
  echo User profile: %USERPROFILE%
  echo.
)

echo ========================================
echo  CADEgg Launcher
echo ========================================
echo Working directory: %ROOT%
echo Mode: %MODE%
echo Log file: %LOG%
echo.

if /I "%MODE%"=="dev" goto DEV_MODE
goto APP_MODE

:APP_MODE
call :TRY_RESTORE_EXISTING_APP
set "RESTORE_CODE=%ERRORLEVEL%"
if "%RESTORE_CODE%"=="0" exit /b 0
if "%RESTORE_CODE%"=="1" goto FAIL

call :ENSURE_BUILT_APP
if errorlevel 1 goto FAIL

set "APP_EXE=%ROOT%\src-tauri\target\debug\cadegg.exe"
if not exist "%APP_EXE%" (
  echo [ERROR] Built app not found: %APP_EXE%
  echo [ERROR] Built app not found: %APP_EXE% >> "%LOG%"
  goto FAIL
)

echo [INFO] Starting built CADEgg app...
echo [INFO] Starting built CADEgg app: %APP_EXE% >> "%LOG%"
start "" "%APP_EXE%"
echo [INFO] App launched. This console can be closed.
exit /b 0

:DEV_MODE
call :ENSURE_TOOLING
if errorlevel 1 goto FAIL

call :ENSURE_NODE_MODULES
if errorlevel 1 goto FAIL

call :CHECK_DEV_PORT 1420
if errorlevel 1 goto FAIL

echo [INFO] Starting CADEgg in dev mode...
echo [INFO] Dev mode uses Vite port 1420 and keeps this console open.
echo [STEP] Running npm.cmd run tauri -- dev... >> "%LOG%"
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "& npm.cmd run tauri -- dev 2>&1 | Tee-Object -FilePath '%LOG%' -Append; exit $LASTEXITCODE"
set "DEV_EXIT=%ERRORLEVEL%"
echo [INFO] tauri dev exit code: %DEV_EXIT% >> "%LOG%"
if not "%DEV_EXIT%"=="0" goto FAIL
exit /b 0

:TRY_RESTORE_EXISTING_APP
tasklist /FI "IMAGENAME eq cadegg.exe" 2>nul | find /I "cadegg.exe" >nul
if errorlevel 1 exit /b 2

echo [INFO] Existing CADEgg process detected. Trying to restore its window...
echo [INFO] Existing CADEgg process detected. >> "%LOG%"
if not exist "%ROOT%\recover-cadegg-window.ps1" (
  echo [ERROR] recover-cadegg-window.ps1 is missing.
  echo [ERROR] recover-cadegg-window.ps1 is missing. >> "%LOG%"
  exit /b 1
)

powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%ROOT%\recover-cadegg-window.ps1"
set "RECOVER_EXIT=%ERRORLEVEL%"
if "%RECOVER_EXIT%"=="0" (
  echo [INFO] Existing CADEgg window restored.
  echo [INFO] Existing CADEgg window restored. >> "%LOG%"
  exit /b 0
)

echo [ERROR] CADEgg is running, but its window could not be restored.
echo Close stale cadegg.exe in Task Manager, then run this launcher again.
tasklist /FI "IMAGENAME eq cadegg.exe"
echo [ERROR] CADEgg is running but could not be restored. >> "%LOG%"
tasklist /FI "IMAGENAME eq cadegg.exe" >> "%LOG%"
exit /b 1

:ENSURE_BUILT_APP
if exist "%ROOT%\src-tauri\target\debug\cadegg.exe" exit /b 0

echo [INFO] Built app is missing. Building debug app first...
echo [INFO] Built app is missing. Building debug app first. >> "%LOG%"
call :ENSURE_TOOLING
if errorlevel 1 exit /b 1
call :ENSURE_NODE_MODULES
if errorlevel 1 exit /b 1

echo [STEP] Running npm.cmd run tauri -- build --debug --no-bundle... >> "%LOG%"
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "& npm.cmd run tauri -- build --debug --no-bundle 2>&1 | Tee-Object -FilePath '%LOG%' -Append; exit $LASTEXITCODE"
exit /b %ERRORLEVEL%

:ENSURE_TOOLING
if exist "%ProgramFiles%\nodejs\npm.cmd" set "PATH=%ProgramFiles%\nodejs;%PATH%"
if exist "%USERPROFILE%\.cargo\bin\cargo.exe" set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

echo [STEP] Checking npm.cmd... >> "%LOG%"
where npm.cmd >> "%LOG%" 2>&1
if errorlevel 1 (
  echo [ERROR] npm.cmd was not found. Install Node.js first.
  echo [ERROR] npm.cmd was not found. >> "%LOG%"
  exit /b 1
)

echo [STEP] Checking cargo.exe... >> "%LOG%"
where cargo.exe >> "%LOG%" 2>&1
if errorlevel 1 (
  echo [ERROR] cargo.exe was not found. Install Rust toolchain first.
  echo [ERROR] cargo.exe was not found. >> "%LOG%"
  exit /b 1
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
exit /b 0

:ENSURE_NODE_MODULES
if exist "%ROOT%\node_modules" exit /b 0

echo [INFO] node_modules was not found. Installing frontend dependencies...
echo [STEP] Running npm.cmd install... >> "%LOG%"
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "& npm.cmd install 2>&1 | Tee-Object -FilePath '%LOG%' -Append; exit $LASTEXITCODE"
exit /b %ERRORLEVEL%

:CHECK_DEV_PORT
set "PORT=%~1"
set "LISTEN_PID="
for /f "usebackq delims=" %%P in (`powershell.exe -NoProfile -Command "$c=Get-NetTCPConnection -LocalPort %PORT% -State Listen -ErrorAction SilentlyContinue; if ($c) { $c | Select-Object -ExpandProperty OwningProcess -First 1 }"`) do set "LISTEN_PID=%%P"

if defined LISTEN_PID (
  echo [ERROR] Dev port %PORT% is listening in process %LISTEN_PID%.
  echo Close the old dev server, or run this launcher without --dev to start the built app.
  echo [ERROR] Dev port %PORT% is listening in process %LISTEN_PID%. >> "%LOG%"
  exit /b 1
)

echo [INFO] Dev port %PORT% has no listener. TIME_WAIT sockets are ignored.
echo [INFO] Dev port %PORT% has no listener. >> "%LOG%"
exit /b 0

:FAIL
echo.
echo [ERROR] Launcher failed. See log: %LOG%
pause
exit /b 1

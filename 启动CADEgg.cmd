@echo off
setlocal EnableExtensions

cd /d "%~dp0"
title CADEgg Launcher

set "ROOT=%CD%"
set "LOG=%ROOT%\cadegg-launch.log"
set "MODE=app"
set "CAD_PREFLIGHT=1"

:PARSE_ARGS
if "%~1"=="" goto ARGS_DONE
if /I "%~1"=="--dev" set "MODE=dev"
if /I "%~1"=="dev" set "MODE=dev"
if /I "%~1"=="--no-cad" set "CAD_PREFLIGHT=0"
if /I "%~1"=="--skip-cad" set "CAD_PREFLIGHT=0"
shift
goto PARSE_ARGS

:ARGS_DONE

> "%LOG%" (
  echo ==== CADEgg launcher %DATE% %TIME% ====
  echo Script: %~f0
  echo Working directory: %ROOT%
  echo Mode: %MODE%
  echo AutoCAD preflight: %CAD_PREFLIGHT%
  echo User profile: %USERPROFILE%
  echo.
)

echo ========================================
echo  CADEgg Launcher
echo ========================================
echo Working directory: %ROOT%
echo Mode: %MODE%
echo AutoCAD preflight: %CAD_PREFLIGHT%
echo Log file: %LOG%
echo.

call :ENSURE_AUTOCAD_READY

if /I "%MODE%"=="dev" goto DEV_MODE
goto APP_MODE

:ENSURE_AUTOCAD_READY
if "%CAD_PREFLIGHT%"=="0" (
  echo [INFO] AutoCAD preflight skipped by command line.
  echo [INFO] AutoCAD preflight skipped by command line. >> "%LOG%"
  exit /b 0
)

tasklist /FI "IMAGENAME eq acad.exe" 2>nul | find /I "acad.exe" >nul
if not errorlevel 1 (
  echo [INFO] AutoCAD is already running.
  echo [INFO] AutoCAD is already running. >> "%LOG%"
  exit /b 0
)

echo [INFO] AutoCAD is not running. Trying to find and start it...
echo [INFO] AutoCAD is not running. Trying to find and start it. >> "%LOG%"
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference='SilentlyContinue'; $progids=@('AutoCAD.Application'); for($m=30; $m -ge 20; $m--){ foreach($s in '.3','.2','.1',''){ $progids += ('AutoCAD.Application.' + $m + $s) } }; $found=$null; foreach($progid in $progids){ $clsidKey=Get-Item -LiteralPath ('Registry::HKEY_CLASSES_ROOT\' + $progid + '\CLSID') -ErrorAction SilentlyContinue; if(-not $clsidKey){ continue }; $clsid=[string]$clsidKey.GetValue(''); if(-not $clsid){ continue }; $serverKey=Get-Item -LiteralPath ('Registry::HKEY_CLASSES_ROOT\CLSID\' + $clsid + '\LocalServer32') -ErrorAction SilentlyContinue; if(-not $serverKey){ continue }; $cmd=[string]$serverKey.GetValue(''); if($cmd -match '\"([^\"]+\.exe)\"|([^\s]+\.exe)'){ $exe=$matches[1]; if(-not $exe){ $exe=$matches[2] }; $exe=[Environment]::ExpandEnvironmentVariables($exe); if(Test-Path -LiteralPath $exe){ $found=$exe; break } } }; if(-not $found){ $cmdPath=Get-Command acad.exe -ErrorAction SilentlyContinue; if($cmdPath){ $found=$cmdPath.Source } }; if(-not $found){ $roots=@(); foreach($drive in Get-PSDrive -PSProvider FileSystem){ foreach($rel in 'Program Files\Autodesk','Program Files (x86)\Autodesk','Autodesk'){ $p=Join-Path $drive.Root $rel; if(Test-Path -LiteralPath $p){ $roots += $p } } }; foreach($root in $roots){ $item=Get-ChildItem -LiteralPath $root -Filter acad.exe -File -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1; if($item){ $found=$item.FullName; break } } }; if(-not $found){ Write-Host '[WARN] AutoCAD executable not found.'; exit 2 }; Write-Host ('[INFO] Starting AutoCAD: ' + $found); Start-Process -FilePath $found | Out-Null; for($i=0; $i -lt 45; $i++){ Start-Sleep -Seconds 1; if(Get-Process acad -ErrorAction SilentlyContinue){ Write-Host '[INFO] AutoCAD process detected.'; exit 0 } }; Write-Host '[WARN] AutoCAD was started, but acad.exe was not detected within 45s.'; exit 3" >> "%LOG%" 2>&1
set "CAD_EXIT=%ERRORLEVEL%"
if "%CAD_EXIT%"=="0" (
  echo [INFO] AutoCAD started and detected.
  echo [INFO] AutoCAD started and detected. >> "%LOG%"
  exit /b 0
)
if "%CAD_EXIT%"=="2" (
  echo [WARN] AutoCAD was not found in registry. CADEgg will still start, but Bridge stays unavailable until AutoCAD is opened.
  echo [WARN] AutoCAD was not found in registry. >> "%LOG%"
  exit /b 0
)
if "%CAD_EXIT%"=="3" (
  echo [WARN] AutoCAD start was requested but not detected within timeout. CADEgg will continue starting.
  echo [WARN] AutoCAD start was requested but not detected within timeout. >> "%LOG%"
  exit /b 0
)

echo [WARN] AutoCAD preflight failed with code %CAD_EXIT%. CADEgg will continue starting.
echo [WARN] AutoCAD preflight failed with code %CAD_EXIT%. >> "%LOG%"
exit /b 0

:APP_MODE
call :APP_REBUILD_NEEDED
set "REBUILD_NEEDED=%ERRORLEVEL%"
if "%REBUILD_NEEDED%"=="1" (
  tasklist /FI "IMAGENAME eq cadegg.exe" 2>nul | find /I "cadegg.exe" >nul
  if not errorlevel 1 (
    echo [ERROR] CADEgg debug app is stale, but cadegg.exe is already running.
    echo Close the existing CADEgg window/process, then run this launcher again so it can rebuild.
    echo [ERROR] CADEgg debug app is stale while cadegg.exe is running. >> "%LOG%"
    tasklist /FI "IMAGENAME eq cadegg.exe"
    tasklist /FI "IMAGENAME eq cadegg.exe" >> "%LOG%"
    goto FAIL
  )
) else (
  call :TRY_RESTORE_EXISTING_APP
  set "RESTORE_CODE=%ERRORLEVEL%"
  if "%RESTORE_CODE%"=="0" exit /b 0
  if "%RESTORE_CODE%"=="1" goto FAIL
)

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
call :APP_REBUILD_NEEDED
if "%ERRORLEVEL%"=="0" exit /b 0

if /I "%APP_BUILD_REASON%"=="missing" (
  echo [INFO] Built app is missing. Building debug app first...
  echo [INFO] Built app is missing. Building debug app first. >> "%LOG%"
) else (
  echo [INFO] Built app is stale. Rebuilding debug app first...
  echo [INFO] Built app is stale. Rebuilding debug app first. >> "%LOG%"
)
call :ENSURE_TOOLING
if errorlevel 1 exit /b 1
call :ENSURE_NODE_MODULES
if errorlevel 1 exit /b 1

echo [STEP] Running npm.cmd run tauri -- build --debug --no-bundle... >> "%LOG%"
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "& npm.cmd run tauri -- build --debug --no-bundle 2>&1 | Tee-Object -FilePath '%LOG%' -Append; exit $LASTEXITCODE"
exit /b %ERRORLEVEL%

:APP_REBUILD_NEEDED
set "APP_BUILD_REASON="
set "APP_EXE=%ROOT%\src-tauri\target\debug\cadegg.exe"
if not exist "%APP_EXE%" (
  set "APP_BUILD_REASON=missing"
  exit /b 1
)

powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference='Stop'; $root='%ROOT%'; $exe=Join-Path $root 'src-tauri\target\debug\cadegg.exe'; $exeTime=(Get-Item -LiteralPath $exe).LastWriteTimeUtc; $paths=@('src','src-tauri\src','data','src-tauri\tauri.conf.json','package.json','package-lock.json','vite.config.ts','tsconfig.json','dist'); $newest=$null; foreach($rel in $paths){ $p=Join-Path $root $rel; if(-not (Test-Path -LiteralPath $p)){ continue }; $item=Get-Item -LiteralPath $p; if($item -is [System.IO.DirectoryInfo]){ $items=Get-ChildItem -LiteralPath $p -Recurse -File -ErrorAction SilentlyContinue } else { $items=@($item) }; foreach($file in $items){ if(-not $newest -or $file.LastWriteTimeUtc -gt $newest){ $newest=$file.LastWriteTimeUtc } } }; if($newest -and $newest -gt $exeTime){ exit 1 }; exit 0" >> "%LOG%" 2>&1
if "%ERRORLEVEL%"=="0" exit /b 0
set "APP_BUILD_REASON=stale"
exit /b 1

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

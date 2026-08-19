# 错峰补测 glm-4.7-flash：智谱「访问量过大」限流在深夜低峰时段通常缓解。
# 由 Windows 计划任务 CADEgg_glm47_retest 在凌晨触发；结果写日志文件。
# 先删除本次任务，若仍全部 429（cargo test 退出码非 0）再安排次日凌晨重试。
$ErrorActionPreference = 'Continue'
$log = "D:\CADEgg\session-log\glm47flash-retest-$(Get-Date -Format 'yyyyMMdd-HHmmss').log"
$env:CARGO_HOME = 'C:\Users\dani3\.cargo'
$env:PATH = "C:\Users\dani3\.cargo\bin;C:\Program Files\Git\bin;$env:PATH"
Set-Location D:\CADEgg
cargo test --manifest-path D:\CADEgg\src-tauri\Cargo.toml retest_model_headless -- --ignored --nocapture *> $log
$exit = $LASTEXITCODE
"RETEST_EXIT_CODE=$exit" | Out-File -Append $log
schtasks /Delete /TN CADEgg_glm47_retest /F | Out-Null
if ($exit -eq 0) {
    "RETEST_PASSED" | Out-File -Append $log
} else {
    $next = (Get-Date).AddDays(1).ToString('yyyy/MM/dd')
    schtasks /Create /TN CADEgg_glm47_retest /TR "powershell -NoProfile -ExecutionPolicy Bypass -File 'D:\CADEgg\scripts\retest-glm47-flash.ps1'" /SC ONCE /SD $next /ST 03:00 /F | Out-File -Append $log
}

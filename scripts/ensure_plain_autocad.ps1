$ErrorActionPreference = 'SilentlyContinue'

function Get-PlainAutoCadProcesses {
    Get-CimInstance Win32_Process -Filter "Name='acad.exe'" |
        Where-Object {
            $_.CommandLine -match '(?i)(^|\s)/product\s+ACAD(?:\s|$)'
        }
}

$plain = @(Get-PlainAutoCadProcesses)
if ($plain.Count -gt 0) {
    Write-Host '[INFO] A pure AutoCAD process (/product ACAD) is already running.'
    exit 0
}

$vertical = @(Get-CimInstance Win32_Process -Filter "Name='acad.exe'" |
    Where-Object {
        $_.CommandLine -and
        $_.CommandLine -notmatch '(?i)(^|\s)/product\s+ACAD(?:\s|$)'
    })
if ($vertical.Count -gt 0) {
    Write-Host '[INFO] Autodesk vertical product detected; it will be ignored. Starting pure AutoCAD.'
}

# Only the unsuffixed AutoCAD ProgID is eligible. AutoCAD.Application.* aliases
# are commonly registered by Civil 3D and other vertical products.
$found = $null
$clsidKey = Get-Item -LiteralPath 'Registry::HKEY_CLASSES_ROOT\AutoCAD.Application\CLSID'
if ($clsidKey) {
    $clsid = [string]$clsidKey.GetValue('')
    if ($clsid) {
        $serverKey = Get-Item -LiteralPath ('Registry::HKEY_CLASSES_ROOT\CLSID\' + $clsid + '\LocalServer32')
        if ($serverKey) {
            $command = [string]$serverKey.GetValue('')
            if ($command -match '"([^"]+\.exe)"|([^\s]+\.exe)') {
                $exe = if ($matches[1]) { $matches[1] } else { $matches[2] }
                $exe = [Environment]::ExpandEnvironmentVariables($exe)
                if ((Split-Path -Leaf $exe) -ieq 'acad.exe' -and (Test-Path -LiteralPath $exe)) {
                    $found = $exe
                }
            }
        }
    }
}

if (-not $found) {
    $commandPath = Get-Command acad.exe
    if ($commandPath) { $found = $commandPath.Source }
}

if (-not $found) {
    $roots = @()
    foreach ($drive in Get-PSDrive -PSProvider FileSystem) {
        foreach ($relative in 'Program Files\Autodesk', 'Program Files (x86)\Autodesk', 'Autodesk') {
            $root = Join-Path $drive.Root $relative
            if (Test-Path -LiteralPath $root) { $roots += $root }
        }
    }
    foreach ($root in $roots) {
        $item = Get-ChildItem -LiteralPath $root -Filter acad.exe -File -Recurse |
            Select-Object -First 1
        if ($item) {
            $found = $item.FullName
            break
        }
    }
}

if (-not $found) {
    Write-Host '[WARN] Pure AutoCAD executable not found.'
    exit 2
}

Write-Host ('[INFO] Starting pure AutoCAD: ' + $found)
Start-Process -FilePath $found -ArgumentList @('/product', 'ACAD', '/Automation') | Out-Null
for ($i = 0; $i -lt 45; $i++) {
    Start-Sleep -Seconds 1
    if (@(Get-PlainAutoCadProcesses).Count -gt 0) {
        Write-Host '[INFO] Pure AutoCAD process detected.'
        exit 0
    }
}

Write-Host '[WARN] AutoCAD was started, but a /product ACAD process was not detected within 45s.'
exit 3

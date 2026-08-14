#![cfg(windows)]

use std::{
    collections::BTreeSet,
    env,
    f64::consts::PI,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
    thread,
    time::Duration,
};

use crate::tools::SessionObject;
use windows::core::{IUnknown, Interface, BSTR, GUID, PCWSTR, VARIANT};
use windows::Win32::System::Com::{
    CLSIDFromProgID, CoInitializeEx, CoUninitialize, IDispatch, COINIT_APARTMENTTHREADED,
    DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT, DISPPARAMS,
};
use windows::Win32::System::Ole::{
    GetActiveObject, SafeArrayCreateVector, SafeArrayDestroy, SafeArrayPutElement,
};
use windows::Win32::System::Variant::{
    VariantClear, VariantGetDoubleElem, VariantGetElementCount, VT_R8,
};

const LOCALE_USER_DEFAULT: u32 = 0x0400;
const IID_NULL: GUID = GUID::zeroed();
const VT_DISPATCH_U16: u16 = 9;
const VT_ARRAY_R8_U16: u16 = 0x2000 | 5;
const DISPID_PROPERTYPUT: i32 = -3;
const RPC_E_CALL_REJECTED: i32 = 0x80010001u32 as i32;
const RPC_E_SERVERCALL_RETRYLATER: i32 = 0x8001010Au32 as i32;
const COM_RETRY_LIMIT: usize = 8;
const COM_RETRY_DELAY_MS: u64 = 120;
const AUTO_ATTACH_WAIT_ROUNDS: usize = 18;
const AUTO_ATTACH_WAIT_MS: u64 = 750;
const BRIDGE_PORT: u16 = 50471;
const BRIDGE_VERSION: &str = "0.3.4.0";
const BRIDGE_BUNDLE_NAME: &str = "CADEggBridge.bundle";
const BRIDGE_DLL_BASENAME: &str = "CADEggBridge";
const BRIDGE_BUILD_STAMP: &str = "bridge-version.txt";
static BRIDGE_INSTALL_RESULT: OnceLock<Result<PathBuf, String>> = OnceLock::new();

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct AutoCadLaunchCandidate {
    exe_path: PathBuf,
    source: String,
}

#[derive(serde::Serialize)]
struct BridgeRequest<'a> {
    command: &'a str,
    args: serde_json::Value,
}

#[derive(serde::Deserialize)]
struct BridgeResponse {
    ok: bool,
    #[allow(dead_code)]
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: serde_json::Value,
}

#[repr(C)]
struct VariantShadow {
    vt: u16,
    _r1: u16,
    _r2: u16,
    _r3: u16,
    data: *mut core::ffi::c_void,
    _pad: usize,
}

unsafe fn variant_as_dispatch(v: &VARIANT) -> Result<IDispatch, String> {
    let shadow: &VariantShadow = &*(v as *const VARIANT as *const VariantShadow);
    if shadow.vt != VT_DISPATCH_U16 {
        return Err(format!("variant is not IDispatch (vt={})", shadow.vt));
    }
    if shadow.data.is_null() {
        return Err("null IDispatch pointer".to_string());
    }
    let borrowed = IDispatch::from_raw_borrowed(&shadow.data)
        .ok_or_else(|| "from_raw_borrowed returned None".to_string())?;
    Ok(borrowed.clone())
}

unsafe fn variant_from_point3d(x: f64, y: f64, z: f64) -> Result<VARIANT, String> {
    let values = [x, y, z];
    let psa = SafeArrayCreateVector(VT_R8, 0, values.len() as u32);
    if psa.is_null() {
        return Err("创建 SAFEARRAY 失败".to_string());
    }

    for (index, value) in values.iter().enumerate() {
        let idx = index as i32;
        if let Err(error) = SafeArrayPutElement(
            psa,
            &idx,
            value as *const f64 as *const core::ffi::c_void,
        ) {
            let _ = SafeArrayDestroy(psa);
            return Err(format!("写入 SAFEARRAY[{index}] 失败: {error}"));
        }
    }

    let mut variant = VARIANT::new();
    let shadow: &mut VariantShadow = &mut *(&mut variant as *mut VARIANT as *mut VariantShadow);
    shadow.vt = VT_ARRAY_R8_U16;
    shadow.data = psa as *mut core::ffi::c_void;
    Ok(variant)
}

fn run_sta<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce() -> Result<R, String> + Send + 'static,
    R: Send + 'static,
{
    thread::spawn(move || unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() {
            return Err(format!("CoInitializeEx failed: 0x{:08X}", hr.0));
        }
        let result = f();
        CoUninitialize();
        result
    })
    .join()
    .map_err(|_| "COM thread panicked".to_string())?
}

/// `run_sta` 的带超时版本。当 AutoCAD 忙、弹对话框或命令行卡在提示符时，
/// `SendCommand` / COM 调用可能永久阻塞，`thread::join()` 也会跟着挂起。
/// 该版本在超时后放弃等待并返回明确错误，避免测试或调用方无限期卡死。
/// 注意：超时后后台线程仍在运行，但其结果会被丢弃（无法安全终止阻塞中的 COM 调用）。
fn run_sta_with_timeout<F, R>(f: F, timeout: Duration) -> Result<R, String>
where
    F: FnOnce() -> Result<R, String> + Send + 'static,
    R: Send + 'static,
{
    let handle = thread::spawn(move || unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() {
            return Err(format!("CoInitializeEx failed: 0x{:08X}", hr.0));
        }
        let result = f();
        CoUninitialize();
        result
    });

    let deadline = std::time::Instant::now() + timeout;
    loop {
        if handle.is_finished() {
            return handle.join().map_err(|_| "COM thread panicked".to_string())?;
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "AutoCAD 调用超时（超过 {:?} 无响应）。AutoCAD 可能正忙、弹出了对话框，或命令行卡在等待输入的提示符。请按 Esc 取消当前命令，或关闭对话框后重试。",
                timeout
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn is_retryable_com_error(error: &windows::core::Error) -> bool {
    matches!(
        error.code().0,
        RPC_E_CALL_REJECTED | RPC_E_SERVERCALL_RETRYLATER
    )
}

fn retry_com<T, F>(label: &str, mut f: F) -> Result<T, String>
where
    F: FnMut() -> windows::core::Result<T>,
{
    let mut last_error: Option<windows::core::Error> = None;
    for attempt in 0..COM_RETRY_LIMIT {
        match f() {
            Ok(value) => return Ok(value),
            Err(error) if is_retryable_com_error(&error) && attempt + 1 < COM_RETRY_LIMIT => {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(
                    COM_RETRY_DELAY_MS * (attempt as u64 + 1),
                ));
            }
            Err(error) => return Err(format!("{label}: {error}")),
        }
    }

    let error = last_error.unwrap_or_else(|| windows::core::Error::from_win32());
    Err(format!(
        "{label}: AutoCAD 当前忙，重试 {} 次后仍失败: {}",
        COM_RETRY_LIMIT, error
    ))
}

fn known_progids() -> Vec<String> {
    let mut progids = vec!["AutoCAD.Application".to_string()];
    for major in (20..=30).rev() {
        progids.push(format!("AutoCAD.Application.{major}.3"));
        progids.push(format!("AutoCAD.Application.{major}.2"));
        progids.push(format!("AutoCAD.Application.{major}.1"));
        progids.push(format!("AutoCAD.Application.{major}"));
    }
    progids
}

fn query_registry_default_value(key_path: &str) -> Option<String> {
    let output = Command::new("reg")
        .args(["query", key_path, "/ve"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some((_, rest)) = trimmed.split_once("REG_EXPAND_SZ") {
            return Some(rest.trim().to_string());
        }
        if let Some((_, rest)) = trimmed.split_once("REG_SZ") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn expand_env_vars(raw: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' {
            if let Some(end) = chars[i + 1..].iter().position(|c| *c == '%') {
                let var_name: String = chars[i + 1..i + 1 + end].iter().collect();
                if let Ok(value) = env::var(&var_name) {
                    out.push_str(&value);
                } else {
                    out.push('%');
                    out.push_str(&var_name);
                    out.push('%');
                }
                i += end + 2;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn parse_exe_path_from_command(command: &str) -> Option<PathBuf> {
    let expanded = expand_env_vars(command.trim());
    if expanded.is_empty() {
        return None;
    }

    let lower = expanded.to_ascii_lowercase();
    let exe_end = lower.find(".exe")?;
    let exe_end = exe_end + 4;
    let candidate = expanded[..exe_end].trim();
    let candidate = if let Some(stripped) = candidate.strip_prefix('"') {
        stripped.trim_end_matches('"').to_string()
    } else {
        candidate.to_string()
    };

    if candidate.is_empty() {
        None
    } else {
        Some(PathBuf::from(candidate))
    }
}

fn candidate_from_registry_progid(progid: &str) -> Option<AutoCadLaunchCandidate> {
    let clsid = query_registry_default_value(&format!(r"HKCR\{}\CLSID", progid))?;
    let command = query_registry_default_value(&format!(r"HKCR\CLSID\{}\LocalServer32", clsid))?;
    let exe_path = parse_exe_path_from_command(&command)?;
    if !exe_path.exists() {
        return None;
    }

    Some(AutoCadLaunchCandidate {
        exe_path,
        source: format!("registry:{progid}"),
    })
}

fn broken_registry_localserver_entries() -> Vec<String> {
    let mut entries = Vec::new();
    for progid in known_progids() {
        let Some(clsid) = query_registry_default_value(&format!(r"HKCR\{}\CLSID", progid)) else {
            continue;
        };
        let Some(command) =
            query_registry_default_value(&format!(r"HKCR\CLSID\{}\LocalServer32", clsid))
        else {
            continue;
        };
        let Some(exe_path) = parse_exe_path_from_command(&command) else {
            continue;
        };
        if !exe_path.exists() {
            entries.push(format!("{progid} -> {}", exe_path.display()));
        }
    }
    entries.sort();
    entries.dedup();
    entries
}

fn keyword_score(name: &str) -> i32 {
    let lower = name.to_ascii_lowercase();
    let mut score = 0;
    if lower.contains("autocad") {
        score += 3;
    }
    if lower.contains("autodesk") {
        score += 2;
    }
    if lower.contains("cad") {
        score += 1;
    }
    score
}

fn should_descend_dir(name: &str, depth: usize) -> bool {
    if depth >= 3 {
        return false;
    }
    if depth >= 1 {
        return true;
    }
    keyword_score(name) > 0
        || matches!(
            name.to_ascii_lowercase().as_str(),
            "program files" | "program files (x86)"
        )
}

fn collect_acad_from_dir(dir: &Path, depth: usize, found: &mut BTreeSet<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().to_string();

        if file_type.is_file() && name.eq_ignore_ascii_case("acad.exe") {
            found.insert(path);
            continue;
        }

        if file_type.is_dir() && should_descend_dir(&name, depth) {
            collect_acad_from_dir(&path, depth + 1, found);
        }
    }
}

fn discover_filesystem_acad_candidates() -> Vec<AutoCadLaunchCandidate> {
    let mut roots: BTreeSet<PathBuf> = BTreeSet::new();
    for key in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(value) = env::var(key) {
            let path = PathBuf::from(value);
            if path.exists() {
                roots.insert(path);
            }
        }
    }
    for drive in b'C'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", drive as char));
        if root.exists() {
            roots.insert(root);
        }
    }

    let mut found = BTreeSet::new();
    for root in roots {
        collect_acad_from_dir(&root, 0, &mut found);
    }

    found
        .into_iter()
        .map(|exe_path| AutoCadLaunchCandidate {
            exe_path,
            source: "filesystem-scan".to_string(),
        })
        .collect()
}

fn discover_acad_launch_candidates() -> Vec<AutoCadLaunchCandidate> {
    let mut ordered = Vec::new();
    let mut seen = BTreeSet::new();

    for progid in known_progids() {
        if let Some(candidate) = candidate_from_registry_progid(&progid) {
            if seen.insert(candidate.exe_path.clone()) {
                ordered.push(candidate);
            }
        }
    }

    let mut scanned = discover_filesystem_acad_candidates();
    scanned.sort_by(|a, b| {
        let a_name = a
            .exe_path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let b_name = b
            .exe_path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        keyword_score(&b_name)
            .cmp(&keyword_score(&a_name))
            .then_with(|| a.exe_path.cmp(&b.exe_path))
    });
    for candidate in scanned {
        if seen.insert(candidate.exe_path.clone()) {
            ordered.push(candidate);
        }
    }

    ordered
}

fn bridge_source_code() -> &'static str {
    include_str!("../autocad_bridge/CADEggBridge.cs")
}

fn bridge_bundle_root() -> Result<PathBuf, String> {
    let appdata = env::var("APPDATA").map_err(|_| "缺少 APPDATA 环境变量".to_string())?;
    Ok(PathBuf::from(appdata)
        .join("Autodesk")
        .join("ApplicationPlugins")
        .join(BRIDGE_BUNDLE_NAME))
}

fn bridge_bundle_contents_dir() -> Result<PathBuf, String> {
    Ok(bridge_bundle_root()?.join("Contents").join("Windows"))
}

fn bridge_installed_dll_path() -> Result<PathBuf, String> {
    Ok(bridge_bundle_contents_dir()?.join(bridge_versioned_dll_name()))
}

fn bridge_installed_stamp_path() -> Result<PathBuf, String> {
    Ok(bridge_bundle_root()?.join(BRIDGE_BUILD_STAMP))
}

fn bridge_build_root() -> Result<PathBuf, String> {
    let appdata = env::var("APPDATA").map_err(|_| "缺少 APPDATA 环境变量".to_string())?;
    Ok(PathBuf::from(appdata).join("CADEgg").join("bridge-build"))
}

fn bridge_build_source_path() -> Result<PathBuf, String> {
    // 固定文件名：csproj 的 Compile Include 指向这个固定名，
    // 版本区分靠 dll/deps.json 文件名（通过 -p:AssemblyName 传入版本号）。
    Ok(bridge_build_root()?.join("CADEggBridge.cs"))
}

fn bridge_build_csproj_path() -> Result<PathBuf, String> {
    Ok(bridge_build_root()?.join("CADEggBridge.csproj"))
}

fn bridge_build_dll_path() -> Result<PathBuf, String> {
    Ok(bridge_build_root()?.join(bridge_versioned_dll_name()))
}

fn bridge_build_deps_path() -> Result<PathBuf, String> {
    Ok(bridge_build_root()?.join(bridge_versioned_deps_name()))
}

fn bridge_versioned_deps_name() -> String {
    format!("{BRIDGE_DLL_BASENAME}-{BRIDGE_VERSION}.deps.json")
}

fn bridge_package_contents_xml() -> String {
    let dll = bridge_versioned_dll_name();
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<ApplicationPackage SchemaVersion="1.0" AppVersion="{version}" ProductCode="{{7E25C0E2-8AF7-4D39-93E9-6B2681A4FBA5}}" Name="CADEggBridge" Description="CADEgg AutoCAD internal bridge" Author="CADEgg">
  <CompanyDetails Name="CADEgg" />
  <Components>
    <ComponentEntry AppName="CADEggBridge" Version="{version}" ModuleName="./Contents/Windows/{dll}" AppDescription="CADEgg AutoCAD bridge" AppType=".Net" LoadOnAutoCADStartup="True" LoadOnAppearance="True">
      <RuntimeRequirements OS="Win64" Platform="AutoCAD*" SeriesMin="R26.0" SeriesMax="R26.9" />
    </ComponentEntry>
  </Components>
</ApplicationPackage>
"#,
        version = BRIDGE_VERSION,
        dll = dll
    )
}

fn bridge_versioned_dll_name() -> String {
    format!("{BRIDGE_DLL_BASENAME}-{BRIDGE_VERSION}.dll")
}

/// 探测 dotnet.exe（.NET SDK 的 CLI 宿主）。
/// 用于 dotnet build 编译 net10.0 目标的 bridge。
fn find_dotnet_exe() -> Result<PathBuf, String> {
    for candidate in [
        r"C:\Program Files\dotnet\dotnet.exe",
        r"C:\Program Files (x86)\dotnet\dotnet.exe",
    ] {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return Ok(path);
        }
    }
    // 回退到 PATH 里的 dotnet
    if let Ok(output) = Command::new("where").arg("dotnet").output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(first) = stdout.lines().next() {
                let path = PathBuf::from(first.trim());
                if path.exists() {
                    return Ok(path);
                }
            }
        }
    }
    Err("未找到 dotnet.exe（.NET SDK）。请安装 .NET 10 SDK。".to_string())
}

/// 生成 bridge 的 csproj 模板。acad_dir 是 AutoCAD 安装目录（含 acmgd.dll 等）。
/// 关键：目标框架 net10.0-windows，产出 .dll + .deps.json。
/// .NET 10 插件必须有 .deps.json 才能被 AutoCAD 2027 的 Autoloader 加载。
fn bridge_csproj_template(acad_dir: &Path) -> String {
    let acad = acad_dir.display();
    format!(
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net10.0-windows</TargetFramework>
    <OutputType>Library</OutputType>
    <PlatformTarget>x64</PlatformTarget>
    <RootNamespace>CADEggBridge</RootNamespace>
    <GenerateDocumentationFile>false</GenerateDocumentationFile>
    <Nullable>disable</Nullable>
    <ImplicitUsings>disable</ImplicitUsings>
    <EnableDefaultCompileItems>false</EnableDefaultCompileItems>
    <AppendTargetFrameworkToOutputPath>false</AppendTargetFrameworkToOutputPath>
    <AppendRuntimeIdentifierToOutputPath>false</AppendRuntimeIdentifierToOutputPath>
  </PropertyGroup>
  <ItemGroup>
    <Compile Include="CADEggBridge.cs" />
  </ItemGroup>
  <ItemGroup>
    <Reference Include="Acmgd"><HintPath>{acad}\acmgd.dll</HintPath><Private>false</Private></Reference>
    <Reference Include="Acdbmgd"><HintPath>{acad}\acdbmgd.dll</HintPath><Private>false</Private></Reference>
    <Reference Include="accoremgd"><HintPath>{acad}\accoremgd.dll</HintPath><Private>false</Private></Reference>
  </ItemGroup>
</Project>
"#
    )
}

fn find_managed_api_dir() -> Result<PathBuf, String> {
    for candidate in discover_acad_launch_candidates() {
        if let Some(parent) = candidate.exe_path.parent() {
            let acmgd = parent.join("acmgd.dll");
            let acdbmgd = parent.join("acdbmgd.dll");
            let accoremgd = parent.join("accoremgd.dll");
            if acmgd.exists() && acdbmgd.exists() && accoremgd.exists() {
                return Ok(parent.to_path_buf());
            }
        }
    }
    Err(
        "未找到可用的 AutoCAD .NET API 目录（缺少 acmgd.dll / acdbmgd.dll / accoremgd.dll）"
            .to_string(),
    )
}

fn build_bridge_dll() -> Result<PathBuf, String> {
    let build_root = bridge_build_root()?;
    fs::create_dir_all(&build_root).map_err(|e| format!("创建 bridge 构建目录失败: {e}"))?;

    // 写 C# 源码（固定文件名，csproj 的 Compile Include 指向它）
    let source_path = bridge_build_source_path()?;
    fs::write(&source_path, bridge_source_code())
        .map_err(|e| format!("写 bridge 源码失败: {e}"))?;

    // 写 csproj（内嵌模板，动态注入 AutoCAD API 目录路径）
    let csproj_path = bridge_build_csproj_path()?;
    let api_dir = find_managed_api_dir()?;
    fs::write(&csproj_path, bridge_csproj_template(&api_dir))
        .map_err(|e| format!("写 bridge csproj 失败: {e}"))?;

    let output_dll = bridge_build_dll_path()?;
    let output_deps = bridge_build_deps_path()?;

    // 用 .NET SDK 的 dotnet build 编译，产出 .dll + .deps.json。
    // .NET 10 插件必须带 .deps.json 才能被 AutoCAD 2027 的 Autoloader 正确加载；
    // 之前用裸 Roslyn csc 只产出 .dll、缺 deps.json，导致 bridge 从未被加载。
    let dotnet = find_dotnet_exe()?;
    let output = Command::new(&dotnet)
        .arg("build")
        .arg(csproj_path.as_os_str())
        .arg("-c")
        .arg("Release")
        .arg(format!("-p:AssemblyName={BRIDGE_DLL_BASENAME}-{BRIDGE_VERSION}"))
        .output()
        .map_err(|e| format!("启动 dotnet build 失败: {e}"))?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "编译 AutoCAD bridge（dotnet build / net10.0-windows）失败。\nstdout:\n{}\nstderr:\n{}",
            stdout.trim(),
            stderr.trim()
        ));
    }

    // dotnet build 输出到 bin/Release/ 下，名称为 AssemblyName 拼上 .dll/.deps.json。
    // 由于 csproj 设置了 AppendTargetFrameworkToOutputPath=false 和
    // AppendRuntimeIdentifierToOutputPath=false，产物就在 bin/Release/ 直接目录下。
    let bin_dir = build_root.join("bin").join("Release");
    let built_dll = bin_dir.join(format!("{BRIDGE_DLL_BASENAME}-{BRIDGE_VERSION}.dll"));
    let built_deps = bin_dir.join(format!("{BRIDGE_DLL_BASENAME}-{BRIDGE_VERSION}.deps.json"));

    if !built_dll.exists() {
        return Err(format!(
            "dotnet build 未产出预期 DLL：{}",
            built_dll.display()
        ));
    }
    // 复制到 build_root 下的统一命名位置，供 install_bridge_bundle 使用。
    fs::copy(&built_dll, &output_dll)
        .map_err(|e| format!("复制 bridge DLL 失败: {e}"))?;
    if built_deps.exists() {
        fs::copy(&built_deps, &output_deps)
            .map_err(|e| format!("复制 bridge deps.json 失败: {e}"))?;
    } else {
        return Err(format!(
            "dotnet build 未产出 deps.json：{}",
            built_deps.display()
        ));
    }

    Ok(output_dll)
}

fn install_bridge_bundle() -> Result<PathBuf, String> {
    let installed_dll = bridge_installed_dll_path()?;
    let installed_stamp = bridge_installed_stamp_path()?;
    if installed_dll.exists()
        && installed_stamp.exists()
        && fs::read_to_string(&installed_stamp).ok().as_deref() == Some(BRIDGE_VERSION)
    {
        return bridge_bundle_root();
    }

    let built_dll = build_bridge_dll()?;
    let contents_dir = bridge_bundle_contents_dir()?;
    fs::create_dir_all(&contents_dir).map_err(|e| format!("创建 bridge bundle 目录失败: {e}"))?;
    fs::copy(&built_dll, &installed_dll).map_err(|e| format!("复制 bridge DLL 失败: {e}"))?;
    // 复制 deps.json（.NET 10 插件被 Autoloader 加载的关键文件）
    let built_deps = bridge_build_deps_path()?;
    if built_deps.exists() {
        let installed_deps = contents_dir.join(bridge_versioned_deps_name());
        fs::copy(&built_deps, &installed_deps)
            .map_err(|e| format!("复制 bridge deps.json 失败: {e}"))?;
    }
    fs::write(
        bridge_bundle_root()?.join("PackageContents.xml"),
        bridge_package_contents_xml(),
    )
    .map_err(|e| format!("写 PackageContents.xml 失败: {e}"))?;
    fs::write(&installed_stamp, BRIDGE_VERSION)
        .map_err(|e| format!("写 bridge 版本戳失败: {e}"))?;

    bridge_bundle_root()
}

fn ensure_bridge_installed_once() -> Result<PathBuf, String> {
    BRIDGE_INSTALL_RESULT
        .get_or_init(install_bridge_bundle)
        .clone()
}

fn bridge_send_request(command: &str, args: serde_json::Value) -> Result<BridgeResponse, String> {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", BRIDGE_PORT))
        .map_err(|e| format!("连接 CADEgg bridge 失败: {e}"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));

    let request = BridgeRequest { command, args };
    let payload =
        serde_json::to_string(&request).map_err(|e| format!("序列化 bridge 请求失败: {e}"))?;
    stream
        .write_all(payload.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .map_err(|e| format!("发送 bridge 请求失败: {e}"))?;
    stream
        .flush()
        .map_err(|e| format!("刷新 bridge 请求失败: {e}"))?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("读取 bridge 响应失败: {e}"))?;
    if line.trim().is_empty() {
        return Err("bridge 响应为空".to_string());
    }
    serde_json::from_str::<BridgeResponse>(line.trim())
        .map_err(|e| format!("解析 bridge 响应失败: {e}"))
}

fn attempt_launch_candidate(
    candidate: &AutoCadLaunchCandidate,
    automation: bool,
) -> Result<(), String> {
    let mut command = Command::new(&candidate.exe_path);
    if automation {
        command.arg("/Automation");
    } else {
        command.arg("/regserver");
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("启动 {} 失败: {}", candidate.exe_path.display(), error))
}

unsafe fn get_autocad() -> Result<IDispatch, String> {
    let progids = known_progids();
    let mut last_error = String::new();
    for progid in &progids {
        let wide: Vec<u16> = progid.encode_utf16().chain(std::iter::once(0)).collect();
        let clsid = match retry_com(&format!("CLSIDFromProgID({progid})"), || {
            CLSIDFromProgID(PCWSTR(wide.as_ptr()))
        }) {
            Ok(clsid) => clsid,
            Err(error) => {
                last_error = error;
                continue;
            }
        };

        let mut unknown: Option<IUnknown> = None;
        match retry_com(&format!("GetActiveObject({progid})"), || {
            GetActiveObject(&clsid, None, &mut unknown)
        }) {
            Ok(()) => {
                let unknown =
                    unknown.ok_or_else(|| format!("GetActiveObject({progid}) 返回 null"))?;
                return unknown
                    .cast::<IDispatch>()
                    .map_err(|e| format!("cast IDispatch 失败: {e}"));
            }
            Err(error) => {
                last_error = error;
            }
        }
    }

    let broken_registry = broken_registry_localserver_entries();
    let candidates = discover_acad_launch_candidates();
    let mut attempted = Vec::new();
    let discovered: Vec<String> = candidates
        .iter()
        .take(5)
        .map(|candidate| format!("{} <- {}", candidate.exe_path.display(), candidate.source))
        .collect();
    for candidate in candidates.into_iter().take(3) {
        attempted.push(format!(
            "{} <- {}",
            candidate.exe_path.display(),
            candidate.source
        ));
        match attempt_launch_candidate(&candidate, true) {
            Ok(_) => {
                for _ in 0..AUTO_ATTACH_WAIT_ROUNDS {
                    thread::sleep(Duration::from_millis(AUTO_ATTACH_WAIT_MS));
                    for progid in &progids {
                        let wide: Vec<u16> =
                            progid.encode_utf16().chain(std::iter::once(0)).collect();
                        let clsid = match retry_com(&format!("CLSIDFromProgID({progid})"), || {
                            CLSIDFromProgID(PCWSTR(wide.as_ptr()))
                        }) {
                            Ok(clsid) => clsid,
                            Err(error) => {
                                last_error = error;
                                continue;
                            }
                        };
                        let mut unknown: Option<IUnknown> = None;
                        match retry_com(&format!("GetActiveObject({progid})"), || {
                            GetActiveObject(&clsid, None, &mut unknown)
                        }) {
                            Ok(()) => {
                                let unknown = unknown.ok_or_else(|| {
                                    format!("GetActiveObject({progid}) 返回 null")
                                })?;
                                return unknown
                                    .cast::<IDispatch>()
                                    .map_err(|e| format!("cast IDispatch 失败: {e}"));
                            }
                            Err(error) => {
                                last_error = error;
                            }
                        }
                    }
                }

                if let Err(error) = attempt_launch_candidate(&candidate, false) {
                    last_error = error;
                    continue;
                }
                thread::sleep(Duration::from_secs(4));
                for _ in 0..AUTO_ATTACH_WAIT_ROUNDS {
                    thread::sleep(Duration::from_millis(AUTO_ATTACH_WAIT_MS));
                    for progid in &progids {
                        let wide: Vec<u16> =
                            progid.encode_utf16().chain(std::iter::once(0)).collect();
                        let clsid = match retry_com(&format!("CLSIDFromProgID({progid})"), || {
                            CLSIDFromProgID(PCWSTR(wide.as_ptr()))
                        }) {
                            Ok(clsid) => clsid,
                            Err(error) => {
                                last_error = error;
                                continue;
                            }
                        };
                        let mut unknown: Option<IUnknown> = None;
                        match retry_com(&format!("GetActiveObject({progid})"), || {
                            GetActiveObject(&clsid, None, &mut unknown)
                        }) {
                            Ok(()) => {
                                let unknown = unknown.ok_or_else(|| {
                                    format!("GetActiveObject({progid}) 返回 null")
                                })?;
                                return unknown
                                    .cast::<IDispatch>()
                                    .map_err(|e| format!("cast IDispatch 失败: {e}"));
                            }
                            Err(error) => {
                                last_error = error;
                            }
                        }
                    }
                }
            }
            Err(error) => {
                last_error = format!("启动 {} 失败: {}", candidate.exe_path.display(), error);
            }
        }
    }

    let attempted_hint = if attempted.is_empty() {
        "未找到可自动启动的 acad.exe 候选路径".to_string()
    } else {
        format!("已尝试自动启动: {}", attempted.join("；"))
    };
    let discovered_hint = if discovered.is_empty() {
        String::new()
    } else {
        format!(" 自动扫描到的 acad.exe: {}。", discovered.join("；"))
    };
    let broken_hint = if broken_registry.is_empty() {
        String::new()
    } else {
        format!(" 检测到失效的 COM 注册: {}。", broken_registry.join("；"))
    };
    Err(format!(
        "未能连接到运行中的 AutoCAD COM 对象。请检查 AutoCAD Automation/COM 注册是否正常，尤其是 ProgID 对应的 LocalServer32 是否仍指向有效 acad.exe。{}。{}{}{}最后一次错误: {}",
        attempted_hint,
        discovered_hint,
        broken_hint,
        if discovered_hint.is_empty() && broken_hint.is_empty() {
            ""
        } else {
            " "
        },
        last_error
    ))
}

unsafe fn get_dispid(d: &IDispatch, name: &str) -> Result<i32, String> {
    let wname: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let name_ptr = PCWSTR(wname.as_ptr());
    let names = [name_ptr];
    let mut dispid = 0i32;
    retry_com(&format!("GetIDsOfNames({name})"), || {
        d.GetIDsOfNames(
            &IID_NULL,
            names.as_ptr(),
            1,
            LOCALE_USER_DEFAULT,
            &mut dispid,
        )
    })?;
    Ok(dispid)
}

unsafe fn get_property(d: &IDispatch, name: &str) -> Result<VARIANT, String> {
    let id = get_dispid(d, name)?;
    let mut result = VARIANT::new();
    let params = DISPPARAMS::default();
    retry_com(&format!("Invoke PROPERTYGET {name}"), || {
        d.Invoke(
            id,
            &IID_NULL,
            LOCALE_USER_DEFAULT,
            DISPATCH_PROPERTYGET,
            &params,
            Some(&mut result),
            None,
            None,
        )
    })?;
    Ok(result)
}

unsafe fn invoke_method(
    d: &IDispatch,
    name: &str,
    args: &mut [VARIANT],
) -> Result<VARIANT, String> {
    let id = get_dispid(d, name)?;
    let mut reversed_args = args.to_vec();
    reversed_args.reverse();
    let mut result = VARIANT::new();
    retry_com(&format!("Invoke METHOD {name}"), || {
        let params = DISPPARAMS {
            rgvarg: reversed_args.as_mut_ptr(),
            rgdispidNamedArgs: std::ptr::null_mut(),
            cArgs: reversed_args.len() as u32,
            cNamedArgs: 0,
        };
        d.Invoke(
            id,
            &IID_NULL,
            LOCALE_USER_DEFAULT,
            DISPATCH_METHOD,
            &params,
            Some(&mut result),
            None,
            None,
        )
    })?;
    Ok(result)
}

unsafe fn put_property(
    d: &IDispatch,
    name: &str,
    value: &mut VARIANT,
) -> Result<(), String> {
    let id = get_dispid(d, name)?;
    let mut dispid_named = DISPID_PROPERTYPUT;
    retry_com(&format!("Invoke PROPERTYPUT {name}"), || {
        let params = DISPPARAMS {
            rgvarg: value as *mut VARIANT,
            rgdispidNamedArgs: &mut dispid_named,
            cArgs: 1,
            cNamedArgs: 1,
        };
        d.Invoke(
            id,
            &IID_NULL,
            LOCALE_USER_DEFAULT,
            DISPATCH_PROPERTYPUT,
            &params,
            None,
            None,
            None,
        )
    })?;
    Ok(())
}

unsafe fn get_active_document(app: &IDispatch) -> Result<IDispatch, String> {
    let doc_v = get_property(app, "ActiveDocument").map_err(|e| {
        format!(
            "{e}（AutoCAD 可能停在开始页，尚未打开任何 DWG 文档；请先新建或打开一个图纸文档）"
        )
    })?;
    variant_as_dispatch(&doc_v)
}

unsafe fn get_model_space(doc: &IDispatch) -> Result<IDispatch, String> {
    let space_v = get_property(doc, "ModelSpace")?;
    variant_as_dispatch(&space_v)
}

unsafe fn get_pickfirst_selection_set(doc: &IDispatch) -> Result<IDispatch, String> {
    let sel_v = get_property(doc, "PickfirstSelectionSet")?;
    variant_as_dispatch(&sel_v)
}

unsafe fn get_bstr_property(d: &IDispatch, name: &str) -> Result<String, String> {
    let value = get_property(d, name)?;
    let bstr = BSTR::try_from(&value).map_err(|e| format!("{name} BSTR 取出失败: {e}"))?;
    Ok(bstr.to_string())
}

unsafe fn get_i32_property(d: &IDispatch, name: &str) -> Result<i32, String> {
    let value = get_property(d, name)?;
    i32::try_from(&value).map_err(|e| format!("{name} 解析为 i32 失败: {e}"))
}

unsafe fn get_f64_property(d: &IDispatch, name: &str) -> Result<f64, String> {
    let mut value = get_property(d, name)?;
    let result = f64::try_from(&value).map_err(|e| format!("{name} 解析为 f64 失败: {e}"));
    let _ = VariantClear(&mut value);
    result
}

unsafe fn get_bool_property(d: &IDispatch, name: &str) -> Result<bool, String> {
    let mut value = get_property(d, name)?;
    let result = bool::try_from(&value)
        .or_else(|_| i16::try_from(&value).map(|v| v != 0))
        .or_else(|_| i32::try_from(&value).map(|v| v != 0))
        .map_err(|e| format!("{name} 解析为 bool 失败: {e}"));
    let _ = VariantClear(&mut value);
    result
}

unsafe fn get_point_property(d: &IDispatch, name: &str) -> Result<(f64, f64, f64), String> {
    let mut value = get_property(d, name)?;
    let count = VariantGetElementCount(&value);
    if count < 3 {
        let _ = VariantClear(&mut value);
        return Err(format!("{name} 点坐标元素不足: {count}"));
    }

    let x = VariantGetDoubleElem(&value, 0).map_err(|e| format!("{name}[0] 读取失败: {e}"));
    let y = VariantGetDoubleElem(&value, 1).map_err(|e| format!("{name}[1] 读取失败: {e}"));
    let z = VariantGetDoubleElem(&value, 2).map_err(|e| format!("{name}[2] 读取失败: {e}"));
    let _ = VariantClear(&mut value);

    Ok((x?, y?, z?))
}

unsafe fn get_f64_array_property(d: &IDispatch, name: &str) -> Result<Vec<f64>, String> {
    let mut value = get_property(d, name)?;
    let count = VariantGetElementCount(&value);
    let mut values = Vec::with_capacity(count as usize);
    for idx in 0..count {
        let num = VariantGetDoubleElem(&value, idx)
            .map_err(|e| format!("{name}[{idx}] 读取失败: {e}"))?;
        values.push(num);
    }
    let _ = VariantClear(&mut value);
    Ok(values)
}

unsafe fn collection_item(d: &IDispatch, index: i32) -> Result<IDispatch, String> {
    let mut args = [VARIANT::from(index)];
    let item_v = invoke_method(d, "Item", &mut args)?;
    variant_as_dispatch(&item_v)
}

unsafe fn wait_for_new_model_space_object(
    doc: &IDispatch,
    baseline_count: i32,
    timeout: Duration,
) -> Result<IDispatch, String> {
    let poll = Duration::from_millis(50);
    let mut waited = Duration::ZERO;
    while waited <= timeout {
        let model_space = get_model_space(doc)?;
        let count = get_i32_property(&model_space, "Count")?;
        if count > baseline_count {
            return collection_item(&model_space, count - 1);
        }
        thread::sleep(poll);
        waited += poll;
    }
    Err("等待新建对象超时，未能读取创建后的 Handle".to_string())
}

unsafe fn ensure_handle_exists(doc: &IDispatch, handle: &str) -> Result<(), String> {
    let bstr = BSTR::from(handle);
    let mut args = [VARIANT::from(bstr)];
    invoke_method(doc, "HandleToObject", &mut args)
        .map(|_| ())
        .map_err(|e| format!("Handle '{handle}' 不存在或无法访问: {e}"))
}

unsafe fn get_object_by_handle(doc: &IDispatch, handle: &str) -> Result<IDispatch, String> {
    let bstr = BSTR::from(handle);
    let mut args = [VARIANT::from(bstr)];
    let obj_v = invoke_method(doc, "HandleToObject", &mut args)?;
    variant_as_dispatch(&obj_v)
}

/// Send a raw command line string to AutoCAD via ActiveDocument.SendCommand.
/// Caller is responsible for newline termination per AutoCAD's command parser.
unsafe fn send_command(app: &IDispatch, cmd: &str) -> Result<(), String> {
    let doc = get_active_document(app)?;
    send_command_to_doc(&doc, cmd)
}

/// 内部版本：向已知 ActiveDocument 发送命令。供批量出图复用同一 doc 句柄，
/// 避免每次 send_command 都重新获取 ActiveDocument 并触发额外的 COM 调用。
unsafe fn send_command_to_doc(doc: &IDispatch, cmd: &str) -> Result<(), String> {
    wait_until_autocad_idle(doc)?;
    let bstr = BSTR::from(cmd);
    let mut args = [VARIANT::from(bstr)];
    invoke_method(doc, "SendCommand", &mut args)?;
    Ok(())
}

unsafe fn add_text_via_com(
    doc: &IDispatch,
    x: f64,
    y: f64,
    text: &str,
    height: f64,
    rotation_deg: f64,
) -> Result<IDispatch, String> {
    wait_until_autocad_idle(doc)?;
    let model_space = get_model_space(doc)?;
    let insertion_point = variant_from_point3d(x, y, 0.0)?;
    let text_value = VARIANT::from(BSTR::from(text));
    let insertion_value = insertion_point;
    let height_value = VARIANT::from(height);
    let mut args = [text_value, insertion_value, height_value];
    let text_object = invoke_method(
        &model_space,
        "AddText",
        &mut args,
    )?;
    let text_dispatch = variant_as_dispatch(&text_object)?;
    if rotation_deg.abs() > f64::EPSILON {
        let mut rotation_value = VARIANT::from(rotation_deg.to_radians());
        put_property(&text_dispatch, "Rotation", &mut rotation_value)?;
    }
    Ok(text_dispatch)
}

/// 等待 AutoCAD 命令行空闲（无正在执行的命令）。
/// 通过读取 CMDNAMES 系统变量判断：命令执行中其值非空，空闲时为空字符串。
/// 若 AutoCAD 一直忙（例如卡在等待输入的提示符、弹出了对话框），超时后返回明确错误。
unsafe fn wait_until_autocad_idle(doc: &IDispatch) -> Result<(), String> {
    const MAX_WAIT: Duration = Duration::from_secs(10);
    let poll = Duration::from_millis(100);
    let mut waited = Duration::ZERO;
    loop {
        match get_variable_string(doc, "CMDNAMES") {
            Ok(cmd_names) if cmd_names.trim().is_empty() => return Ok(()),
            Ok(cmd_names) => {
                if waited >= MAX_WAIT {
                    return Err(format!(
                        "AutoCAD 命令 '{cmd_names}' 仍在执行中，等待空闲超时（{} 秒）。请按 Esc 取消当前命令，或关闭弹出的对话框后重试。",
                        MAX_WAIT.as_secs()
                    ));
                }
            }
            Err(e) => {
                // GetVariable 本身失败（AutoCAD 忙/拒绝），先静默继续轮询。
                // 等到它空闲时再判断 CMDNAMES。
                if waited >= MAX_WAIT {
                    return Err(format!(
                        "等待 AutoCAD 空闲超时（{} 秒），最后错误：{e}。AutoCAD 可能正忙、弹出了对话框，或命令行卡在等待输入的提示符。请按 Esc 取消当前命令后重试。",
                        MAX_WAIT.as_secs()
                    ));
                }
            }
        }
        thread::sleep(poll);
        waited += poll;
    }
}

/// 读取文档系统变量（字符串类型）。用 GetVariable 方法。
unsafe fn get_variable_string(doc: &IDispatch, name: &str) -> Result<String, String> {
    let bstr_name = BSTR::from(name);
    let mut args = [VARIANT::from(bstr_name)];
    let value = invoke_method(doc, "GetVariable", &mut args)?;
    BSTR::try_from(&value)
        .map(|b| b.to_string())
        .or_else(|_| Ok(value.to_string()))
}

fn fmt_num(n: f64) -> String {
    // AutoCAD's command parser doesn't like scientific notation; force fixed point.
    if n.fract() == 0.0 {
        format!("{:.1}", n)
    } else {
        format!("{n}")
    }
}

/// 估算文字在 AutoCAD 中的视觉宽度（绘图单位）。
/// 中日韩全角字符约占 1.0 倍字高，ASCII 半角字符约占 0.55 倍字高。
/// 用于在默认「左基线对齐」的文字上做视觉居中与单元格内定位，
/// 避免依赖 bridge/COM 的 Alignment 属性。
fn estimate_text_width(text: &str, height: f64) -> f64 {
    let mut width = 0.0;
    for ch in text.chars() {
        let code = ch as u32;
        let is_fullwidth = code >= 0x2E80
            || matches!(
                ch,
                '，' | '。' | '（' | '）' | '：' | '；' | '、' | '！' | '？' | '「' | '」'
            );
        width += if is_fullwidth { height } else { 0.55 * height };
    }
    width
}

fn normalize_object_kind(object_name: &str) -> String {
    match object_name {
        "AcDbLine" => "LINE".to_string(),
        "AcDbCircle" => "CIRCLE".to_string(),
        "AcDbArc" => "ARC".to_string(),
        "AcDbPolyline" => "LWPOLYLINE".to_string(),
        "AcDbText" => "TEXT".to_string(),
        "AcDbMText" => "MTEXT".to_string(),
        other => other
            .strip_prefix("AcDb")
            .unwrap_or(other)
            .to_ascii_uppercase(),
    }
}

fn fmt_angle_deg(radians: f64) -> String {
    fmt_num(radians * 180.0 / PI)
}

#[cfg_attr(not(test), allow(dead_code))]
fn extract_first_handle(content: &str) -> Result<String, String> {
    content
        .split("handle=")
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .map(|handle| {
            handle
                .trim_matches(|c: char| c == ',' || c == '，' || c == '。')
                .to_string()
        })
        .filter(|handle| !handle.is_empty())
        .ok_or_else(|| format!("未能从结果中解析 handle: {content}"))
}

#[cfg_attr(not(test), allow(dead_code))]
fn extract_created_handle(content: &str) -> Result<String, String> {
    content
        .split("新对象 handle=")
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .map(|handle| {
            handle
                .trim_matches(|c: char| c == ',' || c == '，' || c == '。')
                .to_string()
        })
        .filter(|handle| !handle.is_empty())
        .ok_or_else(|| format!("未能从结果中解析新对象 handle: {content}"))
}

#[cfg_attr(not(test), allow(dead_code))]
fn extract_created_handle_or_fallback(content: &str, fallback: &str) -> Result<String, String> {
    if content.contains("新对象 handle=") {
        extract_created_handle(content)
    } else {
        Ok(fallback.to_string())
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn line_matches(info: &str, x1: f64, y1: f64, x2: f64, y2: f64) -> bool {
    let forward = format!(
        "直线 ({},{}) → ({},{})",
        fmt_num(x1),
        fmt_num(y1),
        fmt_num(x2),
        fmt_num(y2)
    );
    let backward = format!(
        "直线 ({},{}) → ({},{})",
        fmt_num(x2),
        fmt_num(y2),
        fmt_num(x1),
        fmt_num(y1)
    );
    info.contains(&forward) || info.contains(&backward)
}

fn format_polyline_preview(coords: &[f64]) -> Option<String> {
    let mut points = Vec::new();
    let mut iter = coords.chunks_exact(2);
    for pair in iter.by_ref().take(4) {
        points.push(format!("({},{})", fmt_num(pair[0]), fmt_num(pair[1])));
    }

    if points.is_empty() {
        return None;
    }

    let mut preview = points.join(" → ");
    if iter.next().is_some() {
        preview.push_str(" → ...");
    }
    Some(preview)
}

unsafe fn describe_object_geometry(object: &IDispatch, kind: &str, fallback_label: &str) -> String {
    match kind {
        "LINE" => {
            let start = get_point_property(object, "StartPoint");
            let end = get_point_property(object, "EndPoint");
            match (start, end) {
                (Ok((x1, y1, _)), Ok((x2, y2, _))) => format!(
                    "直线 ({},{}) → ({},{})",
                    fmt_num(x1),
                    fmt_num(y1),
                    fmt_num(x2),
                    fmt_num(y2)
                ),
                _ => fallback_label.to_string(),
            }
        }
        "CIRCLE" => {
            let center = get_point_property(object, "Center");
            let radius = get_f64_property(object, "Radius");
            match (center, radius) {
                (Ok((cx, cy, _)), Ok(r)) => {
                    format!("圆心 ({},{}) 半径 {}", fmt_num(cx), fmt_num(cy), fmt_num(r))
                }
                _ => fallback_label.to_string(),
            }
        }
        "ARC" => {
            let center = get_point_property(object, "Center");
            let radius = get_f64_property(object, "Radius");
            let start_angle = get_f64_property(object, "StartAngle");
            let end_angle = get_f64_property(object, "EndAngle");
            match (center, radius, start_angle, end_angle) {
                (Ok((cx, cy, _)), Ok(r), Ok(start), Ok(end)) => format!(
                    "圆弧 圆心 ({},{}) 半径 {} 角度 {}°→{}°",
                    fmt_num(cx),
                    fmt_num(cy),
                    fmt_num(r),
                    fmt_angle_deg(start),
                    fmt_angle_deg(end)
                ),
                _ => fallback_label.to_string(),
            }
        }
        "LWPOLYLINE" => {
            let coords = get_f64_array_property(object, "Coordinates");
            let closed = get_bool_property(object, "Closed");
            match (coords, closed) {
                (Ok(coords), Ok(closed)) if coords.len() >= 4 => {
                    let vertex_count = coords.len() / 2;
                    let closure = if closed { " 闭合" } else { "" };
                    if let Some(preview) = format_polyline_preview(&coords) {
                        format!("多段线 {} 点{}: {}", vertex_count, closure, preview)
                    } else {
                        fallback_label.to_string()
                    }
                }
                _ => fallback_label.to_string(),
            }
        }
        "TEXT" | "MTEXT" => {
            let insertion = get_point_property(object, "InsertionPoint");
            let content = get_bstr_property(object, "TextString");
            match (insertion, content) {
                (Ok((x, y, _)), Ok(text)) => {
                    format!("文字 \"{}\" @ ({},{})", text, fmt_num(x), fmt_num(y))
                }
                _ => fallback_label.to_string(),
            }
        }
        _ => fallback_label.to_string(),
    }
}

unsafe fn session_object_from_dispatch(object: &IDispatch) -> Result<SessionObject, String> {
    let handle = get_bstr_property(object, "Handle")?;
    let object_name = get_bstr_property(object, "ObjectName")?;
    let kind = normalize_object_kind(&object_name);
    let label = describe_object_geometry(object, &kind, &kind);

    Ok(SessionObject {
        handle,
        kind,
        label,
        source: Some("selection".to_string()),
    })
}

unsafe fn created_object_summary(object: &IDispatch) -> Result<(String, String, String), String> {
    let handle = get_bstr_property(object, "Handle")?;
    let object_name =
        get_bstr_property(object, "ObjectName").unwrap_or_else(|_| "UNKNOWN".to_string());
    let kind = normalize_object_kind(&object_name);
    let label = describe_object_geometry(object, &kind, &kind);
    Ok((handle, kind, label))
}

unsafe fn read_pickfirst_selection_objects(doc: &IDispatch) -> Result<Vec<SessionObject>, String> {
    let sel = get_pickfirst_selection_set(doc)?;
    let count = get_i32_property(&sel, "Count")?;
    if count == 0 {
        return Err("当前未在 CAD 里选中任何对象。请先在 AutoCAD 里圈选目标后再导入。".to_string());
    }

    let mut objects = Vec::with_capacity(count as usize);
    for i in 0..count {
        let item = collection_item(&sel, i)?;
        objects.push(session_object_from_dispatch(&item)?);
    }
    Ok(objects)
}

// ---------------- Existing demo commands (kept for COM test panel) ---------------

#[tauri::command]
pub fn test_cad_connection() -> Result<String, String> {
    let install_result = ensure_bridge_installed_once();
    let install_result_for_com = install_result.clone();
    if let Ok(response) = bridge_send_request("ping", serde_json::json!({})) {
        if response.ok {
            let version = response
                .data
                .get("acad_version")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let doc_name = response
                .data
                .get("document_name")
                .and_then(|value| value.as_str())
                .unwrap_or("未命名");
            return Ok(format!(
                "✓ AutoCAD 已通过内部桥接连接 (Version: {}, Document: {})",
                version, doc_name
            ));
        }
    }

    run_sta(move || unsafe {
        let app = get_autocad()?;
        let version = get_bstr_property(&app, "Version")?;
        let mut msg = format!("✓ AutoCAD 已通过 COM 连接 (Version: {})", version);
        if let Ok(bundle_root) = &install_result_for_com {
            msg.push_str(&format!(
                "；内部桥接已安装到 {}，重启 AutoCAD 后将自动优先启用",
                bundle_root.display()
            ));
        }
        Ok(msg)
    })
    .or_else(|com_error| match install_result {
        Ok(bundle_root) => Err(format!(
            "内部桥接已自动安装到 {}，但当前 AutoCAD 尚未加载 bridge。请重启 AutoCAD 一次后自动启用。原始连接错误：{}",
            bundle_root.display(),
            com_error
        )),
        Err(install_error) => Err(format!(
            "{}；另外自动安装内部桥接也失败：{}",
            com_error, install_error
        )),
    })
}

#[tauri::command]
pub fn draw_test_line() -> Result<String, String> {
    let _ = ensure_bridge_installed_once();
    if let Ok(response) = bridge_send_request(
        "draw_line",
        serde_json::json!({
            "x1": 0.0,
            "y1": 0.0,
            "x2": 1000.0,
            "y2": 0.0
        }),
    ) {
        if response.ok {
            let handle = response
                .data
                .get("handle")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            return Ok(format!(
                "✓ 已通过内部桥接画线 (0,0) → (1000,0)，handle={}",
                handle
            ));
        }
    }

    run_sta(|| unsafe {
        let app = get_autocad()?;
        send_command(&app, "_.LINE\n0,0\n1000,0\n\n")?;
        Ok("✓ 已通过 COM 发送画线指令 (0,0) → (1000,0)".to_string())
    })
}

#[tauri::command]
pub fn undo_last_generation() -> Result<String, String> {
    run_sta(|| unsafe {
        let app = get_autocad()?;
        send_command(&app, "_.UNDO\nB\n")?;
        Ok("已撤回上一次生成（UNDO Back）".to_string())
    })
}

#[tauri::command]
pub fn sync_session_objects(
    session_objects: Vec<SessionObject>,
) -> Result<Vec<SessionObject>, String> {
    if session_objects.is_empty() {
        return Ok(Vec::new());
    }

    run_sta(move || unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        let mut synced = Vec::with_capacity(session_objects.len());

        for object in session_objects {
            let SessionObject {
                handle,
                kind,
                label,
                source,
            } = object;

            let Ok(dispatch) = get_object_by_handle(&doc, &handle) else {
                continue;
            };

            let actual_handle =
                get_bstr_property(&dispatch, "Handle").unwrap_or_else(|_| handle.clone());
            let object_name = get_bstr_property(&dispatch, "ObjectName").unwrap_or(kind);
            let normalized_kind = normalize_object_kind(&object_name);
            let refreshed_label = describe_object_geometry(&dispatch, &normalized_kind, &label);
            synced.push(SessionObject {
                handle: actual_handle,
                kind: normalized_kind,
                label: refreshed_label,
                source,
            });
        }

        Ok(synced)
    })
}

#[tauri::command]
pub fn import_selected_objects() -> Result<Vec<SessionObject>, String> {
    run_sta(|| unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        read_pickfirst_selection_objects(&doc)
    })
}

// ---------------- Parameterized tools (called by tools::dispatch) ----------------

pub fn cad_draw_line(x1: f64, y1: f64, x2: f64, y2: f64) -> Result<String, String> {
    let _ = ensure_bridge_installed_once();
    if let Ok(response) = bridge_send_request(
        "draw_line",
        serde_json::json!({
            "x1": x1,
            "y1": y1,
            "x2": x2,
            "y2": y2
        }),
    ) {
        if response.ok {
            let handle = response
                .data
                .get("handle")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            return Ok(format!(
                "已画直线 ({},{}) → ({},{})，handle={}",
                fmt_num(x1),
                fmt_num(y1),
                fmt_num(x2),
                fmt_num(y2),
                handle
            ));
        }
    }

    run_sta(move || unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        let model_space = get_model_space(&doc)?;
        let before_count = get_i32_property(&model_space, "Count")?;
        let cmd = format!(
            "_.LINE\n{},{}\n{},{}\n\n",
            fmt_num(x1),
            fmt_num(y1),
            fmt_num(x2),
            fmt_num(y2)
        );
        send_command(&app, &cmd)?;
        let obj = wait_for_new_model_space_object(&doc, before_count, Duration::from_secs(3))?;
        let handle = get_bstr_property(&obj, "Handle")?;
        Ok(format!(
            "已画直线 ({},{}) → ({},{})，handle={}",
            fmt_num(x1),
            fmt_num(y1),
            fmt_num(x2),
            fmt_num(y2),
            handle
        ))
    })
}

pub fn cad_draw_circle(cx: f64, cy: f64, r: f64) -> Result<String, String> {
    if r <= 0.0 {
        return Err(format!("半径必须为正数，收到 {r}"));
    }
    let _ = ensure_bridge_installed_once();
    if let Ok(response) = bridge_send_request(
        "draw_circle",
        serde_json::json!({
            "cx": cx,
            "cy": cy,
            "r": r
        }),
    ) {
        if response.ok {
            let handle = response
                .data
                .get("handle")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            return Ok(format!(
                "已画圆 圆心({},{}) 半径{}，handle={}",
                fmt_num(cx),
                fmt_num(cy),
                fmt_num(r),
                handle
            ));
        }
    }

    run_sta(move || unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        let model_space = get_model_space(&doc)?;
        let before_count = get_i32_property(&model_space, "Count")?;
        let cmd = format!(
            "_.CIRCLE\n{},{}\n{}\n",
            fmt_num(cx),
            fmt_num(cy),
            fmt_num(r)
        );
        send_command(&app, &cmd)?;
        let obj = wait_for_new_model_space_object(&doc, before_count, Duration::from_secs(3))?;
        let handle = get_bstr_property(&obj, "Handle")?;
        Ok(format!(
            "已画圆 圆心({},{}) 半径{}，handle={}",
            fmt_num(cx),
            fmt_num(cy),
            fmt_num(r),
            handle
        ))
    })
}

pub fn cad_draw_regular_polygon(
    cx: f64,
    cy: f64,
    sides: i32,
    radius: f64,
    radius_mode: &str,
    rotation_deg: f64,
) -> Result<String, String> {
    if sides < 3 {
        return Err(format!("边数至少为 3，收到 {sides}"));
    }
    if radius <= 0.0 {
        return Err(format!("半径必须为正数，收到 {radius}"));
    }

    let radius_mode_owned = radius_mode.trim().to_string();
    let circumradius = match radius_mode_owned.as_str() {
        "circumradius" => radius,
        "inradius" => radius / (PI / sides as f64).cos(),
        other => {
            return Err(format!(
                "radius_mode 只能是 'circumradius' 或 'inradius'，收到 '{other}'"
            ))
        }
    };

    let rotation_rad = rotation_deg.to_radians();
    let mut cmd = String::from("_.PLINE\n");
    for i in 0..sides {
        let theta = rotation_rad + 2.0 * PI * i as f64 / sides as f64;
        let x = cx + circumradius * theta.cos();
        let y = cy + circumradius * theta.sin();
        cmd.push_str(&format!("{},{}\n", fmt_num(x), fmt_num(y)));
    }
    cmd.push_str("C\n");

    run_sta(move || unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        let model_space = get_model_space(&doc)?;
        let before_count = get_i32_property(&model_space, "Count")?;
        send_command(&app, &cmd)?;
        let obj = wait_for_new_model_space_object(&doc, before_count, Duration::from_secs(3))?;
        let handle = get_bstr_property(&obj, "Handle")?;
        Ok(format!(
            "已画正{}边形 中心({},{}) 半径{}({}) 旋转{}°，handle={}",
            sides,
            fmt_num(cx),
            fmt_num(cy),
            fmt_num(radius),
            radius_mode_owned,
            fmt_num(rotation_deg),
            handle
        ))
    })
}

pub fn cad_draw_equilateral_triangle_about_circle(
    cx: f64,
    cy: f64,
    r: f64,
    relation: &str,
    apex_up: bool,
) -> Result<String, String> {
    let rotation_deg = if apex_up { 90.0 } else { -90.0 };
    let radius_mode = match relation.trim() {
        "incircle" => "inradius",
        "circumcircle" => "circumradius",
        other => {
            return Err(format!(
                "relation 只能是 'incircle' 或 'circumcircle'，收到 '{other}'"
            ))
        }
    };

    cad_draw_regular_polygon(cx, cy, 3, r, radius_mode, rotation_deg).map(|content| {
        format!(
            "已按 relation={} 绘制尖角朝{}的等边三角形；{}",
            relation.trim(),
            if apex_up { "上" } else { "下" },
            content
        )
    })
}

pub fn cad_draw_rectangle_by_center(
    cx: f64,
    cy: f64,
    width: f64,
    height: f64,
    rotation_deg: f64,
) -> Result<String, String> {
    if width <= 0.0 || height <= 0.0 {
        return Err(format!(
            "width 和 height 必须为正数，收到 width={} height={}",
            width, height
        ));
    }

    let hw = width / 2.0;
    let hh = height / 2.0;
    let rotation_rad = rotation_deg.to_radians();
    let base_points = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)];

    let mut cmd = String::from("_.PLINE\n");
    for (x, y) in base_points {
        let rx = x * rotation_rad.cos() - y * rotation_rad.sin();
        let ry = x * rotation_rad.sin() + y * rotation_rad.cos();
        cmd.push_str(&format!("{},{}\n", fmt_num(cx + rx), fmt_num(cy + ry)));
    }
    cmd.push_str("C\n");

    run_sta(move || unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        let model_space = get_model_space(&doc)?;
        let before_count = get_i32_property(&model_space, "Count")?;
        send_command(&app, &cmd)?;
        let obj = wait_for_new_model_space_object(&doc, before_count, Duration::from_secs(3))?;
        let handle = get_bstr_property(&obj, "Handle")?;
        Ok(format!(
            "已画矩形 中心({},{}) 宽{} 高{} 旋转{}°，handle={}",
            fmt_num(cx),
            fmt_num(cy),
            fmt_num(width),
            fmt_num(height),
            fmt_num(rotation_deg),
            handle
        ))
    })
}

pub fn cad_draw_double_flight_stair(
    x: f64,
    y: f64,
    flight_width: f64,
    step_depth: f64,
    steps_per_flight: i32,
    landing_depth: f64,
    turn: &str,
    include_arrow: bool,
    include_label: bool,
) -> Result<String, String> {
    if flight_width <= 0.0 {
        return Err(format!("flight_width 必须为正数，收到 {flight_width}"));
    }
    if step_depth <= 0.0 {
        return Err(format!("step_depth 必须为正数，收到 {step_depth}"));
    }
    if steps_per_flight < 2 {
        return Err(format!("steps_per_flight 至少为 2，收到 {steps_per_flight}"));
    }
    if landing_depth <= 0.0 {
        return Err(format!("landing_depth 必须为正数，收到 {landing_depth}"));
    }

    let side = match turn.trim().to_ascii_lowercase().as_str() {
        "left" => "left".to_string(),
        "right" => "right".to_string(),
        other => return Err(format!("turn 只能是 left 或 right，收到 '{other}'")),
    };

    let run_length = step_depth * steps_per_flight as f64;
    let second_x = if side == "right" {
        x + flight_width
    } else {
        x - flight_width
    };
    let min_x = x.min(second_x);
    let max_x = x.max(second_x) + flight_width;
    let top_y = y + run_length;
    let landing_top_y = top_y + landing_depth;

    let first_x1 = x;
    let first_x2 = x + flight_width;
    let second_x1 = second_x;
    let second_x2 = second_x + flight_width;

    fn push_line(cmd: &mut String, x1: f64, y1: f64, x2: f64, y2: f64) {
        cmd.push_str(&format!(
            "_.LINE\n{},{}\n{},{}\n\n",
            fmt_num(x1),
            fmt_num(y1),
            fmt_num(x2),
            fmt_num(y2)
        ));
    }

    fn push_rect(cmd: &mut String, x1: f64, y1: f64, x2: f64, y2: f64) {
        cmd.push_str("_.PLINE\n");
        cmd.push_str(&format!("{},{}\n", fmt_num(x1), fmt_num(y1)));
        cmd.push_str(&format!("{},{}\n", fmt_num(x2), fmt_num(y1)));
        cmd.push_str(&format!("{},{}\n", fmt_num(x2), fmt_num(y2)));
        cmd.push_str(&format!("{},{}\n", fmt_num(x1), fmt_num(y2)));
        cmd.push_str("C\n");
    }

    let mut cmd = String::new();
    push_rect(&mut cmd, first_x1, y, first_x2, top_y);
    push_rect(&mut cmd, second_x1, y, second_x2, top_y);
    push_rect(&mut cmd, min_x, top_y, max_x, landing_top_y);

    for i in 1..steps_per_flight {
        let ty = y + step_depth * i as f64;
        push_line(&mut cmd, first_x1, ty, first_x2, ty);
        push_line(&mut cmd, second_x1, ty, second_x2, ty);
    }

    if include_arrow {
        let arrow_margin = run_length * 0.18;
        let arrow_head = flight_width.min(run_length) * 0.12;
        let first_cx = (first_x1 + first_x2) / 2.0;
        let second_cx = (second_x1 + second_x2) / 2.0;
        let first_y1 = y + arrow_margin;
        let first_y2 = top_y - arrow_margin;
        let second_y1 = top_y - arrow_margin;
        let second_y2 = y + arrow_margin;

        push_line(&mut cmd, first_cx, first_y1, first_cx, first_y2);
        push_line(
            &mut cmd,
            first_cx,
            first_y2,
            first_cx - arrow_head,
            first_y2 - arrow_head,
        );
        push_line(
            &mut cmd,
            first_cx,
            first_y2,
            first_cx + arrow_head,
            first_y2 - arrow_head,
        );

        push_line(&mut cmd, second_cx, second_y1, second_cx, second_y2);
        push_line(
            &mut cmd,
            second_cx,
            second_y2,
            second_cx - arrow_head,
            second_y2 + arrow_head,
        );
        push_line(
            &mut cmd,
            second_cx,
            second_y2,
            second_cx + arrow_head,
            second_y2 + arrow_head,
        );
    }

    if include_label {
        let text_height = (flight_width * 0.12).max(120.0);
        cmd.push_str(&format!(
            "_.TEXT\n{},{}\n{}\n0\nUP\n",
            fmt_num((first_x1 + first_x2) / 2.0),
            fmt_num(y + run_length * 0.5),
            fmt_num(text_height)
        ));
    }

    run_sta(move || unsafe {
        let app = get_autocad()?;
        send_command(&app, &cmd)?;
        Ok(format!(
            "已生成双跑楼梯 平面起点({},{}) 跑宽{} 踏步深{} 每跑{}级 平台深{} 转向{}",
            fmt_num(x),
            fmt_num(y),
            fmt_num(flight_width),
            fmt_num(step_depth),
            steps_per_flight,
            fmt_num(landing_depth),
            side
        ))
    })
}

pub fn cad_draw_elevator_shaft_protection(
    x: f64,
    y: f64,
    opening_width: f64,
    opening_height: f64,
    guardrail_height: f64,
    post_spacing: f64,
    toe_board_height: f64,
    include_warning_sign: bool,
    include_material_table: bool,
    scale: f64,
) -> Result<String, String> {
    if opening_width <= 0.0 || opening_height <= 0.0 {
        return Err(format!(
            "opening_width 和 opening_height 必须为正数，收到 width={} height={}",
            opening_width, opening_height
        ));
    }
    if guardrail_height <= 0.0 || post_spacing <= 0.0 || toe_board_height <= 0.0 {
        return Err(format!(
            "guardrail_height、post_spacing、toe_board_height 必须为正数，收到 {}, {}, {}",
            guardrail_height, post_spacing, toe_board_height
        ));
    }
    if scale <= 0.0 {
        return Err(format!("scale 必须为正数，收到 {scale}"));
    }

    fn push_line(cmd: &mut String, x1: f64, y1: f64, x2: f64, y2: f64) {
        cmd.push_str(&format!(
            "_.LINE\n{},{}\n{},{}\n\n",
            fmt_num(x1),
            fmt_num(y1),
            fmt_num(x2),
            fmt_num(y2)
        ));
    }

    fn push_rect(cmd: &mut String, x1: f64, y1: f64, x2: f64, y2: f64) {
        cmd.push_str("_.PLINE\n");
        cmd.push_str(&format!("{},{}\n", fmt_num(x1), fmt_num(y1)));
        cmd.push_str(&format!("{},{}\n", fmt_num(x2), fmt_num(y1)));
        cmd.push_str(&format!("{},{}\n", fmt_num(x2), fmt_num(y2)));
        cmd.push_str(&format!("{},{}\n", fmt_num(x1), fmt_num(y2)));
        cmd.push_str("C\n");
    }

    let width = opening_width * scale;
    let height = opening_height * scale;
    let rail_h = guardrail_height * scale;
    let toe_h = toe_board_height * scale;
    let max_post_spacing = post_spacing * scale;
    let margin = 450.0 * scale;
    let post_size = (90.0 * scale).max(40.0);
    let rail_gap = (180.0 * scale).max(80.0);

    // ── 字号分层：不同文字类型用不同高度，避免「一刀切过大」 ──
    // 标题 / 说明 / 尺寸标注 / 表头 / 表格正文 / 警示文字
    let title_h = (200.0 * scale).max(120.0); // 图面主标题
    let note_h = (120.0 * scale).max(72.0); // 说明文字
    let dim_h = (100.0 * scale).max(60.0); // 尺寸标注
    let header_h = (110.0 * scale).max(64.0); // 表格表头
    let cell_h = (95.0 * scale).max(56.0); // 表格正文
    let sign_h_text = (130.0 * scale).max(72.0); // 警示牌文字

    let left = x - width / 2.0;
    let right = x + width / 2.0;
    let bottom = y - height / 2.0;
    let top = y + height / 2.0;
    let guard_left = left - margin;
    let guard_right = right + margin;
    let guard_bottom = bottom - margin;
    let guard_top = top + margin;

    // ── 顶部纵向布局：标题 / 说明 / 警示牌自下而上排列，避免互相遮挡 ──
    // 各区块之间的垂直间隙，按字号留足安全距离。
    let title_y = guard_top + 260.0 * scale;
    let note_y = title_y + title_h + 160.0 * scale;
    let sign_y1 = note_y + note_h + 160.0 * scale;
    // 警示牌框：宽高动态匹配警示文字宽度 + 内边距，避免文字溢出框外。
    let sign_text = "当心坠落 禁止跨越".to_string();
    let sign_pad = 80.0 * scale; // 框内左右各留的内边距
    let sign_w = estimate_text_width(&sign_text, sign_h_text) + sign_pad * 2.0;
    let sign_h = sign_h_text + sign_pad * 2.0;
    let sign_x1 = x - sign_w / 2.0;

    // ── 材料表布局常量：表头 1 行 + 数据 4 行，两列 ──
    // 表格放在护栏右侧，顶部与护栏顶对齐；第二列加宽以容纳长文字。
    // 表格左边界向右移，避开右侧「井口高」垂直尺寸标注文字（否则水平重叠）。
    let table_x = guard_right + 1300.0 * scale;
    let table_y = guard_top;
    let table_rows: usize = 5;
    let row_h = 240.0 * scale;
    let col_w0 = 720.0 * scale; // 第一列：材料
    let col_w1 = 1100.0 * scale; // 第二列：数量/规格

    // 单元格中心 X（用于文字水平居中）：第 col 列的中心横坐标
    let cell_center_x = |col: usize| -> f64 {
        if col == 0 {
            table_x + col_w0 / 2.0
        } else {
            table_x + col_w0 + col_w1 / 2.0
        }
    };
    // 单元格中心 Y（第 row 行，0 为表头）：用于文字垂直居中
    let cell_center_y = |row: usize| -> f64 { table_y - row_h * (row as f64 + 0.5) };

    let side_post_count = ((guard_top - guard_bottom) / max_post_spacing).ceil().max(1.0) as i32;
    let top_post_count = ((guard_right - guard_left) / max_post_spacing).ceil().max(1.0) as i32;
    // 实际立杆数：顶/底边各 (top_post_count+1) 根，左右侧边各 (side_post_count-1) 根。
    // 材料表必须用这个真实值，不能再用「周长÷间距」估算（两者会不一致）。
    let post_count =
        ((top_post_count + 1) * 2 + side_post_count.saturating_sub(1) * 2) as i32;

    // 分批发送几何命令，文字单独通过 cad_draw_text 创建。
    // 原因：一次性 send_command 几十条命令时，长串后半段会被 AutoCAD 截断或破坏；
    // 文字若走命令行 TEXT 还可能把 AutoCAD 留在输入提示符中。

    // ── 批 1：所有矩形（PLINE）──
    let mut cmd_rects = String::new();
    push_rect(&mut cmd_rects, left, bottom, right, top); // 井口轮廓
    push_rect(&mut cmd_rects, guard_left, guard_bottom, guard_right, guard_top); // 护栏外框
    // 踢脚板：井口底部独立的横跨矮矩形带（高度 toe_h，宽度 = 井口宽），
    // 而不是贴在护栏外框四边的细条（细条会和护栏框视觉重合看不出）。
    push_rect(&mut cmd_rects, left, bottom, right, bottom + toe_h);
    for i in 0..=top_post_count {
        let t = i as f64 / top_post_count as f64;
        let px = guard_left + (guard_right - guard_left) * t;
        push_rect(
            &mut cmd_rects,
            px - post_size / 2.0,
            guard_bottom - post_size / 2.0,
            px + post_size / 2.0,
            guard_bottom + post_size / 2.0,
        );
        push_rect(
            &mut cmd_rects,
            px - post_size / 2.0,
            guard_top - post_size / 2.0,
            px + post_size / 2.0,
            guard_top + post_size / 2.0,
        );
    }
    for i in 1..side_post_count {
        let t = i as f64 / side_post_count as f64;
        let py = guard_bottom + (guard_top - guard_bottom) * t;
        push_rect(
            &mut cmd_rects,
            guard_left - post_size / 2.0,
            py - post_size / 2.0,
            guard_left + post_size / 2.0,
            py + post_size / 2.0,
        );
        push_rect(
            &mut cmd_rects,
            guard_right - post_size / 2.0,
            py - post_size / 2.0,
            guard_right + post_size / 2.0,
            py + post_size / 2.0,
        );
    }
    if include_warning_sign {
        push_rect(&mut cmd_rects, sign_x1, sign_y1, sign_x1 + sign_w, sign_y1 + sign_h);
    }

    // ── 批 2：所有直线（LINE）──
    let mut cmd_lines = String::new();
    let mid_offset = rail_h * 0.5;
    push_line(&mut cmd_lines, guard_left, guard_bottom + rail_gap, guard_right, guard_bottom + rail_gap);
    push_line(&mut cmd_lines, guard_left, guard_bottom + rail_gap + mid_offset, guard_right, guard_bottom + rail_gap + mid_offset);
    push_line(&mut cmd_lines, guard_left, guard_top - rail_gap, guard_right, guard_top - rail_gap);
    push_line(&mut cmd_lines, guard_left, guard_top - rail_gap - mid_offset, guard_right, guard_top - rail_gap - mid_offset);
    push_line(&mut cmd_lines, guard_left + rail_gap, guard_bottom, guard_left + rail_gap, guard_top);
    push_line(&mut cmd_lines, guard_left + rail_gap + mid_offset, guard_bottom, guard_left + rail_gap + mid_offset, guard_top);
    push_line(&mut cmd_lines, guard_right - rail_gap, guard_bottom, guard_right - rail_gap, guard_top);
    push_line(&mut cmd_lines, guard_right - rail_gap - mid_offset, guard_bottom, guard_right - rail_gap - mid_offset, guard_top);
    let dim_y = guard_bottom - 300.0 * scale;
    let dim_x = guard_right + 300.0 * scale;
    push_line(&mut cmd_lines, left, dim_y, right, dim_y);
    push_line(&mut cmd_lines, left, dim_y - 60.0 * scale, left, dim_y + 60.0 * scale);
    push_line(&mut cmd_lines, right, dim_y - 60.0 * scale, right, dim_y + 60.0 * scale);
    push_line(&mut cmd_lines, dim_x, bottom, dim_x, top);
    push_line(&mut cmd_lines, dim_x - 60.0 * scale, bottom, dim_x + 60.0 * scale, bottom);
    push_line(&mut cmd_lines, dim_x - 60.0 * scale, top, dim_x + 60.0 * scale, top);
    if include_material_table {
        let table_width = col_w0 + col_w1;
        // 行线：5 行（表头 + 4 数据行），共 6 条横线
        for row in 0..=table_rows {
            let y0 = table_y - row_h * row as f64;
            push_line(&mut cmd_lines, table_x, y0, table_x + table_width, y0);
        }
        // 列线：2 列，共 3 条竖线，贯穿全部 5 行（表头行同样分列）
        let x_mid = table_x + col_w0;
        push_line(&mut cmd_lines, x_mid, table_y, x_mid, table_y - row_h * table_rows as f64);
    }

    // ── 批 3：所有文字（bridge / COM AddText）──
    // 文字含中文，SendCommand 经 BSTR 传递可能损坏导致不创建，
    // 改为收集坐标后逐条调用 cad_draw_text（内部 bridge 优先，DBText 直接建中文）。
    // 布局策略：
    //   - 字号分层：标题 title_h / 说明 note_h / 标注 dim_h / 表头 header_h / 正文 cell_h / 警示 sign_h_text
    //   - 尺寸标注：水平标注放在井口下方居中对齐，垂直标注放在右侧居中对齐
    //   - 标题 + 说明：纵向排列在护栏上方，互不遮挡
    //   - 表格文字：按 cell 中心定位（水平/垂直居中，估算文字宽度后左移半宽）
    //   - 警示牌文字：在警示牌框内居中
    let mut text_items: Vec<(f64, f64, f64, String)> = Vec::new();

    // 尺寸标注（水平：井口宽，居中对齐在井口下方）
    let dim_width_text = format!("井口宽 {}", fmt_num(opening_width));
    let dim_width_x = x - estimate_text_width(&dim_width_text, dim_h) / 2.0;
    text_items.push((dim_width_x, dim_y - 160.0 * scale, dim_h, dim_width_text));

    // 尺寸标注（垂直：井口高，居中对齐在井口右侧）
    let dim_height_text = format!("井口高 {}", fmt_num(opening_height));
    let dim_height_x = dim_x + 120.0 * scale;
    // 垂直居中：用字高的一半做偏移（不是文字宽度的一半），让文字落在标注线中点
    let dim_height_y = y - dim_h / 2.0;
    text_items.push((dim_height_x, dim_height_y, dim_h, dim_height_text));

    // 主标题（居中对齐在护栏上方）
    let title_text = "电梯井口临边防护".to_string();
    let title_x = x - estimate_text_width(&title_text, title_h) / 2.0;
    text_items.push((title_x, title_y, title_h, title_text));

    // 说明文字（居中对齐在标题上方）
    let note_text = format!(
        "防护高度 H={}  立杆间距 ≤ {}  踢脚板 {}",
        fmt_num(guardrail_height),
        fmt_num(post_spacing),
        fmt_num(toe_board_height)
    );
    let note_x = x - estimate_text_width(&note_text, note_h) / 2.0;
    text_items.push((note_x, note_y, note_h, note_text));

    if include_warning_sign {
        // 警示牌文字在框内水平+垂直居中；sign_w/sign_h 已在顶部按文字宽度动态计算
        let sign_text_w = estimate_text_width(&sign_text, sign_h_text);
        let sign_tx = sign_x1 + (sign_w - sign_text_w) / 2.0;
        let sign_ty = sign_y1 + (sign_h - sign_h_text) / 2.0;
        text_items.push((sign_tx, sign_ty, sign_h_text, sign_text.clone()));
    }

    if include_material_table {
        // 表头行（第 0 行，居中）
        let header_material = "材料".to_string();
        let header_qty = "数量/规格".to_string();
        let hx0 = cell_center_x(0) - estimate_text_width(&header_material, header_h) / 2.0;
        let hx1 = cell_center_x(1) - estimate_text_width(&header_qty, header_h) / 2.0;
        text_items.push((hx0, cell_center_y(0) - header_h / 2.0, header_h, header_material));
        text_items.push((hx1, cell_center_y(0) - header_h / 2.0, header_h, header_qty));

        // 数据行（第 1~4 行），每格文字居中
        let data_rows: [(String, String); 4] = [
            ("立杆".to_string(), format!("{} 根", post_count)),
            ("横杆".to_string(), "上横杆+中横杆".to_string()),
            ("踢脚板".to_string(), format!("高 {}", fmt_num(toe_board_height))),
            (
                "警示牌".to_string(),
                if include_warning_sign { "1 块".to_string() } else { "未绘制".to_string() },
            ),
        ];
        for (row_idx, (label, value)) in data_rows.iter().enumerate() {
            let row = row_idx + 1; // 数据行从第 1 行开始
            let lx = cell_center_x(0) - estimate_text_width(label, cell_h) / 2.0;
            let vx = cell_center_x(1) - estimate_text_width(value, cell_h) / 2.0;
            let cy = cell_center_y(row) - cell_h / 2.0;
            text_items.push((lx, cy, cell_h, label.clone()));
            text_items.push((vx, cy, cell_h, value.clone()));
        }
    }

    // 先画几何（矩形 + 线，纯 ASCII 坐标，SendCommand 可靠）。
    run_sta_with_timeout(
        move || unsafe {
            let app = get_autocad()?;
            let doc = get_active_document(&app)?;
            send_command_to_doc(&doc, &cmd_rects)?;
            send_command_to_doc(&doc, &cmd_lines)?;
            Ok(())
        },
        Duration::from_secs(60),
    )?;

    // 再逐条画文字（bridge 优先；失败时用 COM AddText，避免命令行 TEXT）。
    for (tx, ty, th, tt) in &text_items {
        cad_draw_text(*tx, *ty, tt, *th, 0.0)?;
    }

    Ok(format!(
        "已生成电梯井口临边防护布置：中心({},{})，井口 {}x{}，防护高度 {}，立杆间距 <= {}，踢脚板 {}，警示牌={}，材料表={}。",
        fmt_num(x),
        fmt_num(y),
        fmt_num(opening_width),
        fmt_num(opening_height),
        fmt_num(guardrail_height),
        fmt_num(post_spacing),
        fmt_num(toe_board_height),
        include_warning_sign,
        include_material_table
    ))
}

pub fn cad_validate_elevator_shaft_protection(
    opening_width: f64,
    opening_height: f64,
    guardrail_height: f64,
    post_spacing: f64,
    toe_board_height: f64,
    include_warning_sign: bool,
    include_material_table: bool,
) -> Result<String, String> {
    let mut checks = Vec::new();
    let mut issues = Vec::new();

    macro_rules! add_check {
        ($id:expr, $label:expr, $passed:expr) => {{
            let passed = $passed;
            checks.push(serde_json::json!({
                "id": $id,
                "label": $label,
                "passed": passed
            }));
            if !passed {
                issues.push($label.to_string());
            }
        }};
    }

    add_check!(
        "opening_size_valid",
        "井口宽度和高度已提供且为正数",
        opening_width > 0.0 && opening_height > 0.0
    );
    add_check!(
        "guardrail_height_valid",
        "防护栏杆高度不低于 1200mm",
        guardrail_height >= 1200.0
    );
    add_check!(
        "post_spacing_valid",
        "立杆间距不大于 2000mm",
        post_spacing > 0.0 && post_spacing <= 2000.0
    );
    add_check!(
        "toe_board_present",
        "踢脚板高度不低于 180mm",
        toe_board_height >= 180.0
    );
    add_check!("warning_sign_present", "警示牌已配置", include_warning_sign);
    add_check!("material_table_present", "材料表已配置", include_material_table);
    add_check!(
        "dimension_complete",
        "井口尺寸、防护高度、立杆间距、踢脚板高度标注齐全",
        opening_width > 0.0
            && opening_height > 0.0
            && guardrail_height > 0.0
            && post_spacing > 0.0
            && toe_board_height > 0.0
    );

    // 立杆数：与绘制逻辑保持一致（顶/底边各 N+1 根，左右侧边各 N-1 根）。
    // 这里用 900mm 边距（margin=450*2）对应绘图的护栏外框尺寸。
    let guard_width = opening_width + 900.0;
    let guard_height = opening_height + 900.0;
    let post_count = if post_spacing > 0.0 {
        let top_post_count = (guard_width / post_spacing).ceil().max(1.0) as i32;
        let side_post_count = (guard_height / post_spacing).ceil().max(1.0) as i32;
        ((top_post_count + 1) * 2 + side_post_count.saturating_sub(1) * 2) as i32
    } else {
        0
    };

    let payload = serde_json::json!({
        "ok": issues.is_empty(),
        "issues": issues,
        "checks": checks,
        "material_table": {
            "posts": post_count,
            "rails": "top_and_mid_rails",
            "toe_board_height": toe_board_height,
            "warning_sign": include_warning_sign,
            "material_table_included": include_material_table
        }
    });
    serde_json::to_string_pretty(&payload).map_err(|e| format!("序列化校核结果失败: {e}"))
}

pub fn cad_draw_text(
    x: f64,
    y: f64,
    text: &str,
    height: f64,
    rotation_deg: f64,
) -> Result<String, String> {
    if height <= 0.0 {
        return Err(format!("文字高度必须为正数，收到 {height}"));
    }
    let text = text.trim().replace('\n', " ");
    if text.is_empty() {
        return Err("文字内容不能为空".to_string());
    }

    let text_for_com = text.clone();
    let _ = ensure_bridge_installed_once();
    let bridge_result = bridge_send_request(
        "draw_text",
        serde_json::json!({
            "x": x,
            "y": y,
            "text": text,
            "height": height,
            "rotation": rotation_deg
        }),
    );
    let bridge_error = match bridge_result {
        Ok(response) if response.ok => {
            let handle = response
                .data
                .get("handle")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            return Ok(format!(
                "已画文字 \"{}\" @ ({},{}) 高{} 旋转{}°，handle={}",
                text,
                fmt_num(x),
                fmt_num(y),
                fmt_num(height),
                fmt_num(rotation_deg),
                handle
            ));
        }
        Ok(response) => Some(format!("bridge draw_text failed: {}", response.message)),
        Err(error) => Some(error),
    };

    let fallback = run_sta_with_timeout(
        move || unsafe {
            let app = get_autocad()?;
            let doc = get_active_document(&app)?;
            let text_obj = add_text_via_com(&doc, x, y, &text_for_com, height, rotation_deg)?;
            let handle = get_bstr_property(&text_obj, "Handle")?;
            Ok(format!(
                "已画文字 \"{}\" @ ({},{}) 高{} 旋转{}°，handle={}",
                text,
                fmt_num(x),
                fmt_num(y),
                fmt_num(height),
                fmt_num(rotation_deg),
                handle
            ))
        },
        Duration::from_secs(15),
    );

    match fallback {
        Ok(result) => Ok(result),
        Err(error) => {
            if let Some(bridge_error) = bridge_error {
                Err(format!("{error}；bridge 也失败：{bridge_error}"))
            } else {
                Err(error)
            }
        }
    }
}

pub fn cad_erase_last() -> Result<String, String> {
    run_sta(|| unsafe {
        let app = get_autocad()?;
        send_command(&app, "_.ERASE\nL\n\n")?;
        Ok("已删除最后绘制的对象".to_string())
    })
}

pub fn cad_erase_handle(handle: &str) -> Result<String, String> {
    let handle = handle.trim().to_string();
    if handle.is_empty() {
        return Err("handle 不能为空".to_string());
    }
    let _ = ensure_bridge_installed_once();
    if let Ok(response) = bridge_send_request(
        "erase_handle",
        serde_json::json!({ "handle": handle.clone() }),
    ) {
        if response.ok {
            let actual_handle = response
                .data
                .get("handle")
                .and_then(|value| value.as_str())
                .unwrap_or(&handle);
            return Ok(format!("已删除 handle={} 的对象", actual_handle));
        }
    }

    run_sta(move || unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        ensure_handle_exists(&doc, &handle)?;
        let cmd = format!("(command \"_.ERASE\" (handent \"{handle}\") \"\")\n");
        send_command(&app, &cmd)?;
        Ok(format!("已删除 handle={} 的对象", handle))
    })
}

pub fn cad_move(dx: f64, dy: f64, target: &str) -> Result<String, String> {
    let sel = match target {
        "last" => "L",
        "previous" => "P",
        other => {
            return Err(format!(
                "target 只能是 'last' 或 'previous'，收到 '{other}'"
            ))
        }
    };
    let target_owned = target.to_string();
    run_sta(move || unsafe {
        let app = get_autocad()?;
        let cmd = format!("_.MOVE\n{}\n\n0,0\n{},{}\n", sel, fmt_num(dx), fmt_num(dy));
        send_command(&app, &cmd)?;
        Ok(format!(
            "已将 {} 对象平移 ({}, {})",
            target_owned,
            fmt_num(dx),
            fmt_num(dy)
        ))
    })
}

pub fn cad_move_handle(handle: &str, dx: f64, dy: f64) -> Result<String, String> {
    let handle = handle.trim().to_string();
    if handle.is_empty() {
        return Err("handle 不能为空".to_string());
    }
    run_sta(move || unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        ensure_handle_exists(&doc, &handle)?;
        let cmd = format!(
            "(command \"_.MOVE\" (handent \"{handle}\") \"\" '(0 0 0) '({} {} 0))\n",
            fmt_num(dx),
            fmt_num(dy)
        );
        send_command(&app, &cmd)?;
        Ok(format!(
            "已将 handle={} 对象平移 ({}, {})",
            handle,
            fmt_num(dx),
            fmt_num(dy)
        ))
    })
}

pub fn cad_rotate_handle(handle: &str, cx: f64, cy: f64, angle_deg: f64) -> Result<String, String> {
    let handle = handle.trim().to_string();
    if handle.is_empty() {
        return Err("handle 不能为空".to_string());
    }
    run_sta(move || unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        ensure_handle_exists(&doc, &handle)?;
        let cmd = format!(
            "(command \"_.ROTATE\" (handent \"{handle}\") \"\" '({} {} 0) {})\n",
            fmt_num(cx),
            fmt_num(cy),
            fmt_num(angle_deg)
        );
        send_command(&app, &cmd)?;
        Ok(format!(
            "已将 handle={} 围绕 ({},{}) 旋转 {}°",
            handle,
            fmt_num(cx),
            fmt_num(cy),
            fmt_num(angle_deg)
        ))
    })
}

pub fn cad_copy_handle(handle: &str, dx: f64, dy: f64) -> Result<String, String> {
    let handle = handle.trim().to_string();
    if handle.is_empty() {
        return Err("handle 不能为空".to_string());
    }
    run_sta(move || unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        ensure_handle_exists(&doc, &handle)?;
        let model_space = get_model_space(&doc)?;
        let before_count = get_i32_property(&model_space, "Count")?;
        let cmd = format!(
            "(command \"_.COPY\" (handent \"{handle}\") \"\" '(0 0 0) '({} {} 0))\n",
            fmt_num(dx),
            fmt_num(dy)
        );
        send_command(&app, &cmd)?;
        let obj = wait_for_new_model_space_object(&doc, before_count, Duration::from_secs(3))?;
        let (new_handle, kind, label) = created_object_summary(&obj)?;
        Ok(format!(
            "已复制 handle={} 的对象并平移 ({}, {})，新对象 handle={} type={} label={}",
            handle,
            fmt_num(dx),
            fmt_num(dy),
            new_handle,
            kind,
            label
        ))
    })
}

pub fn cad_mirror_handle(
    handle: &str,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
) -> Result<String, String> {
    let handle = handle.trim().to_string();
    if handle.is_empty() {
        return Err("handle 不能为空".to_string());
    }
    if x1 == x2 && y1 == y2 {
        return Err("镜像轴的两个点不能相同".to_string());
    }

    let _ = ensure_bridge_installed_once();
    if let Ok(response) = bridge_send_request(
        "mirror_handle",
        serde_json::json!({
            "handle": handle.clone(),
            "x1": x1,
            "y1": y1,
            "x2": x2,
            "y2": y2
        }),
    ) {
        if response.ok {
            let new_handle = response
                .data
                .get("handle")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let kind = response
                .data
                .get("kind")
                .and_then(|value| value.as_str())
                .unwrap_or("UNKNOWN");
            let label = response
                .data
                .get("label")
                .and_then(|value| value.as_str())
                .unwrap_or(kind);
            return Ok(format!(
                "已将 handle={} 的对象按镜像轴 ({},{})→({},{}) 生成镜像副本，新对象 handle={} type={} label={}",
                handle,
                fmt_num(x1),
                fmt_num(y1),
                fmt_num(x2),
                fmt_num(y2),
                new_handle,
                kind,
                label
            ));
        }
    }

    run_sta(move || unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        ensure_handle_exists(&doc, &handle)?;
        let model_space = get_model_space(&doc)?;
        let before_count = get_i32_property(&model_space, "Count")?;
        let cmd = format!(
            "(command \"_.MIRROR\" (handent \"{handle}\") \"\" '({} {} 0) '({} {} 0) \"_N\")\n",
            fmt_num(x1),
            fmt_num(y1),
            fmt_num(x2),
            fmt_num(y2)
        );
        send_command(&app, &cmd)?;
        let obj = wait_for_new_model_space_object(&doc, before_count, Duration::from_secs(3))?;
        let (new_handle, kind, label) = created_object_summary(&obj)?;
        Ok(format!(
            "已将 handle={} 的对象按镜像轴 ({},{})→({},{}) 生成镜像副本，新对象 handle={} type={} label={}",
            handle,
            fmt_num(x1),
            fmt_num(y1),
            fmt_num(x2),
            fmt_num(y2),
            new_handle,
            kind,
            label
        ))
    })
}

pub fn cad_offset_handle(
    handle: &str,
    distance: f64,
    side_x: f64,
    side_y: f64,
) -> Result<String, String> {
    let handle = handle.trim().to_string();
    if handle.is_empty() {
        return Err("handle 不能为空".to_string());
    }
    if distance <= 0.0 {
        return Err(format!("offset 距离必须为正数，收到 {distance}"));
    }

    let _ = ensure_bridge_installed_once();
    if let Ok(response) = bridge_send_request(
        "offset_handle",
        serde_json::json!({
            "handle": handle.clone(),
            "distance": distance,
            "side_x": side_x,
            "side_y": side_y
        }),
    ) {
        if response.ok {
            let new_handle = response
                .data
                .get("handle")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let kind = response
                .data
                .get("kind")
                .and_then(|value| value.as_str())
                .unwrap_or("UNKNOWN");
            let label = response
                .data
                .get("label")
                .and_then(|value| value.as_str())
                .unwrap_or(kind);
            return Ok(format!(
                "已将 handle={} 的对象按距离 {} 向点 ({},{}) 一侧偏移，新对象 handle={} type={} label={}",
                handle,
                fmt_num(distance),
                fmt_num(side_x),
                fmt_num(side_y),
                new_handle,
                kind,
                label
            ));
        }
    }

    run_sta(move || unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        ensure_handle_exists(&doc, &handle)?;
        let model_space = get_model_space(&doc)?;
        let before_count = get_i32_property(&model_space, "Count")?;
        let cmd = format!(
            "(command \"_.OFFSET\" {} (handent \"{handle}\") '({} {} 0) \"\")\n",
            fmt_num(distance),
            fmt_num(side_x),
            fmt_num(side_y)
        );
        send_command(&app, &cmd)?;
        let obj = wait_for_new_model_space_object(&doc, before_count, Duration::from_secs(3))?;
        let (new_handle, kind, label) = created_object_summary(&obj)?;
        Ok(format!(
            "已将 handle={} 的对象按距离 {} 向点 ({},{}) 一侧偏移，新对象 handle={} type={} label={}",
            handle,
            fmt_num(distance),
            fmt_num(side_x),
            fmt_num(side_y),
            new_handle,
            kind,
            label
        ))
    })
}

pub fn cad_trim_by_handle(
    boundary_handle: &str,
    target_handle: &str,
    pick_x: f64,
    pick_y: f64,
) -> Result<String, String> {
    let boundary_handle = boundary_handle.trim().to_string();
    let target_handle = target_handle.trim().to_string();
    if boundary_handle.is_empty() || target_handle.is_empty() {
        return Err("boundary_handle 和 target_handle 都不能为空".to_string());
    }
    if boundary_handle == target_handle {
        return Err("boundary_handle 和 target_handle 不能相同".to_string());
    }

    let _ = ensure_bridge_installed_once();
    if let Ok(response) = bridge_send_request(
        "trim_by_handle",
        serde_json::json!({
            "boundary_handle": boundary_handle.clone(),
            "target_handle": target_handle.clone(),
            "pick_x": pick_x,
            "pick_y": pick_y
        }),
    ) {
        if response.ok {
            let new_handle = response
                .data
                .get("handle")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let kind = response
                .data
                .get("kind")
                .and_then(|value| value.as_str())
                .unwrap_or("UNKNOWN");
            let label = response
                .data
                .get("label")
                .and_then(|value| value.as_str())
                .unwrap_or(kind);
            return Ok(format!(
                "已使用 boundary_handle={} 修剪 target_handle={}，拾取点为 ({},{})，新对象 handle={} type={} label={}",
                boundary_handle,
                target_handle,
                fmt_num(pick_x),
                fmt_num(pick_y)
                ,
                new_handle,
                kind,
                label
            ));
        }
        return Err(format!("trim bridge failed: {}", response.message));
    }

    run_sta(move || unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        ensure_handle_exists(&doc, &boundary_handle)?;
        ensure_handle_exists(&doc, &target_handle)?;
        let cmd = format!(
            "(command \"_.TRIM\" (handent \"{boundary_handle}\") \"\" (handent \"{target_handle}\") '({} {} 0) \"\")\n",
            fmt_num(pick_x),
            fmt_num(pick_y)
        );
        send_command(&app, &cmd)?;
        Ok(format!(
            "已使用 boundary_handle={} 修剪 target_handle={}，拾取点为 ({},{})",
            boundary_handle,
            target_handle,
            fmt_num(pick_x),
            fmt_num(pick_y)
        ))
    })
}

pub fn cad_extend_by_handle(
    boundary_handle: &str,
    target_handle: &str,
    pick_x: f64,
    pick_y: f64,
) -> Result<String, String> {
    let boundary_handle = boundary_handle.trim().to_string();
    let target_handle = target_handle.trim().to_string();
    if boundary_handle.is_empty() || target_handle.is_empty() {
        return Err("boundary_handle 和 target_handle 都不能为空".to_string());
    }
    if boundary_handle == target_handle {
        return Err("boundary_handle 和 target_handle 不能相同".to_string());
    }

    let _ = ensure_bridge_installed_once();
    if let Ok(response) = bridge_send_request(
        "extend_by_handle",
        serde_json::json!({
            "boundary_handle": boundary_handle.clone(),
            "target_handle": target_handle.clone(),
            "pick_x": pick_x,
            "pick_y": pick_y
        }),
    ) {
        if response.ok {
            let new_handle = response
                .data
                .get("handle")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let kind = response
                .data
                .get("kind")
                .and_then(|value| value.as_str())
                .unwrap_or("UNKNOWN");
            let label = response
                .data
                .get("label")
                .and_then(|value| value.as_str())
                .unwrap_or(kind);
            return Ok(format!(
                "已使用 boundary_handle={} 延伸 target_handle={}，拾取点为 ({},{})，新对象 handle={} type={} label={}",
                boundary_handle,
                target_handle,
                fmt_num(pick_x),
                fmt_num(pick_y)
                ,
                new_handle,
                kind,
                label
            ));
        }
        return Err(format!("extend bridge failed: {}", response.message));
    }

    run_sta(move || unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        ensure_handle_exists(&doc, &boundary_handle)?;
        ensure_handle_exists(&doc, &target_handle)?;
        let cmd = format!(
            "(command \"_.EXTEND\" (handent \"{boundary_handle}\") \"\" (handent \"{target_handle}\") '({} {} 0) \"\")\n",
            fmt_num(pick_x),
            fmt_num(pick_y)
        );
        send_command(&app, &cmd)?;
        Ok(format!(
            "已使用 boundary_handle={} 延伸 target_handle={}，拾取点为 ({},{})",
            boundary_handle,
            target_handle,
            fmt_num(pick_x),
            fmt_num(pick_y)
        ))
    })
}

pub fn cad_zoom_extents() -> Result<String, String> {
    run_sta(|| unsafe {
        let app = get_autocad()?;
        send_command(&app, "_.ZOOM\nE\n")?;
        Ok("已缩放至全图范围".to_string())
    })
}

/// Inspect ActiveDocument.PickfirstSelectionSet (objects user has pre-selected
/// in CAD before invoking the agent). Returns a tally by ObjectName.
pub fn cad_list_selection() -> Result<String, String> {
    run_sta(|| unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        let sel = get_pickfirst_selection_set(&doc)?;
        let count = get_i32_property(&sel, "Count")?;
        if count == 0 {
            return Ok(
                "当前未在 CAD 里选中任何对象。请先在 AutoCAD 里圈选目标后再询问。".to_string(),
            );
        }

        let mut tally: std::collections::BTreeMap<String, i32> = std::collections::BTreeMap::new();
        for i in 0..count {
            let item = collection_item(&sel, i)?;
            let name =
                get_bstr_property(&item, "ObjectName").unwrap_or_else(|_| "Unknown".to_string());
            *tally.entry(name).or_insert(0) += 1;
        }
        let parts: Vec<String> = tally.iter().map(|(k, v)| format!("{v}×{k}")).collect();
        Ok(format!("共 {} 个对象: {}", count, parts.join(", ")))
    })
}

/// 图面快照：枚举模型空间所有对象，输出结构化报告。
/// 让后端（agent）能独立"看"到 CAD 里画了什么，用于自动化审查与验收，
/// 不必依赖人工目视。报告含：对象总数、类型分布、每个对象的 handle/类型/图层/颜色/几何。
pub fn cad_modelspace_snapshot() -> Result<String, String> {
    run_sta_with_timeout(
        || unsafe {
            let app = get_autocad()?;
            let doc = get_active_document(&app)?;
            let model_space = get_model_space(&doc)?;
            let count = get_i32_property(&model_space, "Count")?;

            if count == 0 {
                return Ok("图面快照：模型空间为空（0 个对象）。".to_string());
            }

            let mut tally: std::collections::BTreeMap<String, i32> = std::collections::BTreeMap::new();
            let mut details: Vec<String> = Vec::with_capacity(count as usize);
            let mut min_x = f64::INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut max_y = f64::NEG_INFINITY;

            for i in 0..count {
                let item = match collection_item(&model_space, i) {
                    Ok(it) => it,
                    Err(_) => continue,
                };
                let object_name = get_bstr_property(&item, "ObjectName")
                    .unwrap_or_else(|_| "Unknown".to_string());
                let kind = normalize_object_kind(&object_name);
                *tally.entry(kind.clone()).or_insert(0) += 1;

                let handle = get_bstr_property(&item, "Handle").unwrap_or_else(|_| "?".to_string());
                let layer = get_bstr_property(&item, "Layer").unwrap_or_else(|_| "?".to_string());
                let color = get_i32_property(&item, "Color").unwrap_or(-1);
                let geom = describe_object_geometry(&item, &kind, "");

                // 收集几何包围盒（能取到坐标的类型）
                if kind == "LINE" {
                    if let (Ok((x1, y1, _)), Ok((x2, y2, _))) =
                        (get_point_property(&item, "StartPoint"), get_point_property(&item, "EndPoint"))
                    {
                        min_x = min_x.min(x1).min(x2);
                        max_x = max_x.max(x1).max(x2);
                        min_y = min_y.min(y1).min(y2);
                        max_y = max_y.max(y1).max(y2);
                    }
                } else if kind == "CIRCLE" {
                    if let (Ok((cx, cy, _)), Ok(r)) =
                        (get_point_property(&item, "Center"), get_f64_property(&item, "Radius"))
                    {
                        min_x = min_x.min(cx - r);
                        max_x = max_x.max(cx + r);
                        min_y = min_y.min(cy - r);
                        max_y = max_y.max(cy + r);
                    }
                } else if kind == "TEXT" || kind == "MTEXT" {
                    if let Ok((x, y, _)) = get_point_property(&item, "InsertionPoint") {
                        min_x = min_x.min(x);
                        max_x = max_x.max(x);
                        min_y = min_y.min(y);
                        max_y = max_y.max(y);
                    }
                } else if kind == "LWPOLYLINE" {
                    if let Ok(coords) = get_f64_array_property(&item, "Coordinates") {
                        for chunk in coords.chunks(2) {
                            let (px, py) = (chunk[0], chunk[1]);
                            min_x = min_x.min(px);
                            max_x = max_x.max(px);
                            min_y = min_y.min(py);
                            max_y = max_y.max(py);
                        }
                    }
                }

                let mut line = format!("#{i} {kind} handle={handle} 图层={layer} 颜色={color}");
                if !geom.is_empty() {
                    line.push_str(&format!(" | {geom}"));
                }
                details.push(line);
            }

            let mut parts: Vec<String> = tally
                .iter()
                .map(|(k, v)| format!("{v}×{k}"))
                .collect();
            parts.sort();
            let mut report = format!(
                "图面快照：模型空间共 {} 个对象（{}）。\n",
                count,
                parts.join("、")
            );
            if min_x != f64::INFINITY {
                report.push_str(&format!(
                    "整体包围盒：X [{} ~ {}]，Y [{} ~ {}]\n",
                    fmt_num(min_x),
                    fmt_num(max_x),
                    fmt_num(min_y),
                    fmt_num(max_y)
                ));
            }
            report.push_str("对象明细：\n");
            for line in details {
                report.push_str(&line);
                report.push('\n');
            }
            Ok(report)
        },
        Duration::from_secs(30),
    )
}

pub fn cad_run_lisp(code: &str) -> Result<String, String> {
    let code = code.trim();
    if code.is_empty() {
        return Err("LISP 代码为空".to_string());
    }
    let trimmed = code.to_string();
    run_sta(move || unsafe {
        let app = get_autocad()?;
        let cmd = if trimmed.starts_with('(') {
            format!("{trimmed}\n")
        } else {
            format!("({trimmed})\n")
        };
        send_command(&app, &cmd)?;
        Ok("LISP 表达式已下发到 AutoCAD（结果/错误请在 CAD 命令行查看；后端无法回读）".to_string())
    })
}

pub fn cad_inspect_handle(handle: &str) -> Result<String, String> {
    let handle = handle.trim().to_string();
    if handle.is_empty() {
        return Err("handle 不能为空".to_string());
    }
    let _ = ensure_bridge_installed_once();
    if let Ok(response) = bridge_send_request(
        "inspect_handle",
        serde_json::json!({ "handle": handle.clone() }),
    ) {
        if response.ok {
            let actual_handle = response
                .data
                .get("handle")
                .and_then(|value| value.as_str())
                .unwrap_or(&handle);
            let kind = response
                .data
                .get("kind")
                .and_then(|value| value.as_str())
                .unwrap_or("UNKNOWN");
            let label = response
                .data
                .get("label")
                .and_then(|value| value.as_str())
                .unwrap_or(kind);
            return Ok(format!("handle={} type={} {}", actual_handle, kind, label));
        }
    }

    run_sta(move || unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        let object = get_object_by_handle(&doc, &handle)?;
        let object_name =
            get_bstr_property(&object, "ObjectName").unwrap_or_else(|_| "UNKNOWN".to_string());
        let kind = normalize_object_kind(&object_name);
        let label = describe_object_geometry(&object, &kind, &kind);
        Ok(format!("handle={} type={} {}", handle, kind, label))
    })
}

pub fn cad_import_selection() -> Result<Vec<SessionObject>, String> {
    run_sta(|| unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        read_pickfirst_selection_objects(&doc)
    })
}

pub fn cad_begin_undo_group() -> Result<(), String> {
    run_sta(|| unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        let mut no_args: [VARIANT; 0] = [];
        invoke_method(&doc, "StartUndoMark", &mut no_args)?;
        Ok(())
    })
}

pub fn cad_end_undo_group() -> Result<(), String> {
    run_sta(|| unsafe {
        let app = get_autocad()?;
        let doc = get_active_document(&app)?;
        let mut no_args: [VARIANT; 0] = [];
        invoke_method(&doc, "EndUndoMark", &mut no_args)?;
        Ok(())
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn cad_smoke_test_editing_tools() -> Result<String, String> {
    let mut cleanup_handles = Vec::new();

    let result = (|| -> Result<String, String> {
        let boundary = extract_first_handle(&cad_draw_line(100.0, -100.0, 100.0, 100.0)?)?;
        cleanup_handles.push(boundary.clone());
        let trim_target = extract_first_handle(&cad_draw_line(0.0, 50.0, 200.0, 50.0)?)?;
        let trim_result = cad_trim_by_handle(&boundary, &trim_target, 150.0, 50.0)?;
        let trimmed_handle = extract_created_handle_or_fallback(&trim_result, &trim_target)?;
        cleanup_handles.push(trimmed_handle.clone());
        thread::sleep(Duration::from_millis(250));
        let trimmed = cad_inspect_handle(&trimmed_handle)?;
        if !line_matches(&trimmed, 100.0, 50.0, 200.0, 50.0) {
            return Err(format!("trim 校验失败: {trimmed}"));
        }

        let extend_target = extract_first_handle(&cad_draw_line(0.0, -50.0, 80.0, -50.0)?)?;
        let extend_result = cad_extend_by_handle(
            &boundary,
            &extend_target,
            80.0,
            -50.0,
        )?;
        let extended_handle = extract_created_handle_or_fallback(&extend_result, &extend_target)?;
        cleanup_handles.push(extended_handle.clone());
        thread::sleep(Duration::from_millis(250));
        let extended = cad_inspect_handle(&extended_handle)?;
        if !line_matches(&extended, 0.0, -50.0, 100.0, -50.0) {
            return Err(format!("extend 校验失败: {extended}"));
        }

        let mirror_source = extract_first_handle(&cad_draw_line(20.0, 10.0, 40.0, 10.0)?)?;
        cleanup_handles.push(mirror_source.clone());
        let mirrored_handle =
            extract_created_handle(&cad_mirror_handle(&mirror_source, 0.0, -100.0, 0.0, 100.0)?)?;
        cleanup_handles.push(mirrored_handle.clone());
        let mirrored = cad_inspect_handle(&mirrored_handle)?;
        if !line_matches(&mirrored, -20.0, 10.0, -40.0, 10.0) {
            return Err(format!("mirror 校验失败: {mirrored}"));
        }

        let circle_handle = extract_first_handle(&cad_draw_circle(300.0, 0.0, 20.0)?)?;
        cleanup_handles.push(circle_handle.clone());
        let offset_handle =
            extract_created_handle(&cad_offset_handle(&circle_handle, 10.0, 340.0, 0.0)?)?;
        cleanup_handles.push(offset_handle.clone());
        let offset = cad_inspect_handle(&offset_handle)?;
        if !offset.contains("type=CIRCLE") || !offset.contains("圆心 (300.0,0.0) 半径 30.0") {
            return Err(format!("offset 校验失败: {offset}"));
        }

        Ok(format!(
            "真实 AutoCAD 冒烟测试通过：trim={}；extend={}；mirror={}；offset={}",
            trim_target, extend_target, mirrored_handle, offset_handle
        ))
    })();

    for handle in cleanup_handles.iter().rev() {
        let _ = cad_erase_handle(handle);
    }

    result
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn cad_smoke_test_elevator_shaft_protection() -> Result<String, String> {
    let draw_result = cad_draw_elevator_shaft_protection(
        1000.0,
        2000.0,
        2000.0,
        1800.0,
        1200.0,
        2000.0,
        180.0,
        true,
        true,
        1.0,
    )?;
    if !draw_result.contains("电梯井口临边防护布置") {
        return Err(format!("绘图结果不符合预期: {draw_result}"));
    }

    let validation = cad_validate_elevator_shaft_protection(
        2000.0,
        1800.0,
        1200.0,
        2000.0,
        180.0,
        true,
        true,
    )?;
    let validation_json: serde_json::Value =
        serde_json::from_str(&validation).map_err(|e| format!("校核 JSON 解析失败: {e}"))?;
    if validation_json["ok"] != serde_json::Value::Bool(true) {
        return Err(format!("校核未通过: {validation}"));
    }

    Ok(format!(
        "电梯井口临边防护 smoke test 通过：draw={} validation={}",
        draw_result, validation
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        bridge_installed_dll_path, cad_draw_text, cad_modelspace_snapshot,
        cad_smoke_test_editing_tools, cad_smoke_test_elevator_shaft_protection,
        ensure_bridge_installed_once,
    };

    #[test]
    #[ignore = "requires a running AutoCAD session"]
    fn modelspace_snapshot_reports_objects() {
        let report = cad_modelspace_snapshot();
        assert!(report.is_ok(), "{}", report.err().unwrap_or_default());
        let text = report.unwrap();
        assert!(
            text.contains("图面快照") && text.contains("对象"),
            "快照报告格式异常: {text}"
        );
    }

    #[test]
    #[ignore = "requires local AutoCAD managed assemblies and writes ApplicationPlugins bundle"]
    fn bridge_bundle_can_build_and_install() {
        let bundle_root = ensure_bridge_installed_once().expect("bridge install should succeed");
        assert!(bundle_root.exists(), "bundle root should exist");
        assert!(
            bridge_installed_dll_path()
                .map(|path| path.exists())
                .unwrap_or(false),
            "installed bridge dll should exist"
        );
    }

    #[test]
    #[ignore = "requires a running AutoCAD session"]
    fn editing_tools_smoke_test_round_trip() {
        let result = cad_smoke_test_editing_tools();
        assert!(result.is_ok(), "{}", result.err().unwrap_or_default());
    }

    #[test]
    #[ignore = "requires a running AutoCAD session"]
    fn text_english_diagnostic() {
        // 诊断：cad_draw_text 是否能在 bridge 不可用时通过 COM AddText 创建文字。
        // 该路径不应触发命令行 TEXT，也不应让 AutoCAD 卡在文字输入提示符。
        let result = cad_draw_text(0.0, 0.0, "TEST_ABC_123", 200.0, 0.0);
        assert!(result.is_ok(), "{}", result.err().unwrap_or_default());
    }

    #[test]
    #[ignore = "requires a running AutoCAD session"]
    fn elevator_shaft_protection_smoke_test_round_trip() {
        let result = cad_smoke_test_elevator_shaft_protection();
        assert!(result.is_ok(), "{}", result.err().unwrap_or_default());
    }

    #[test]
    fn estimate_text_width_fullwidth_vs_ascii() {
        // 全角中文字符 ≈ 1.0 字高，ASCII 半角 ≈ 0.55 字高
        let h = 100.0;
        let all_cjk = super::estimate_text_width("井口宽", h);
        let all_ascii = super::estimate_text_width("ABC", h);
        assert!((all_cjk - 300.0).abs() < 1e-6, "3 个全角字符应约 300，得到 {all_cjk}");
        assert!((all_ascii - 165.0).abs() < 1e-6, "3 个半角字符应约 165，得到 {all_ascii}");
        // 全角 > 半角
        assert!(all_cjk > all_ascii);
        // 空字符串宽度为 0
        assert_eq!(super::estimate_text_width("", h), 0.0);
    }
}

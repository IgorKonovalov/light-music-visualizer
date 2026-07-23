# Builds foo_lmv.dll (x64 Release): the foobar2000 component that wraps
# lmv-core's C ABI. Requires VS Build Tools 2022 (C++ workload), rustup, and
# the foobar2000 SDK unpacked at plugin-foobar/sdk (see README.md).
#
#   .\build.ps1            # build plugin-foobar/build/foo_lmv.dll
#   .\build.ps1 -Install   # ...then copy it into the foobar2000 v2 profile

param([switch]$Install)

# Native tools (cargo, msbuild) write progress to stderr; exit codes are the
# real failure signal, so keep PowerShell from promoting stderr to errors.
$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Split-Path -Parent $root
$sdk = Join-Path $root "sdk"
$build = Join-Path $root "build"

if (-not (Test-Path (Join-Path $sdk "foobar2000\SDK\foobar2000.h"))) {
    throw "foobar2000 SDK not found at $sdk - see plugin-foobar/README.md"
}

# --- 1. Rust core: release staticlib (fat LTO per the workspace profile) ---
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
cargo build --release -p lmv-core 2>&1 | ForEach-Object { "$_" }
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
$coreLib = Join-Path $repo "target\release\lmv_core.lib"

# --- 2. Locate MSVC ---
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vsroot = & $vswhere -latest -products * `
    -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
    -property installationPath
if (-not $vsroot) { throw "MSVC build tools not found" }
$msbuild = Join-Path $vsroot "MSBuild\Current\Bin\MSBuild.exe"
$vcvars = Join-Path $vsroot "VC\Auxiliary\Build\vcvars64.bat"

# --- 3. SDK static libs (Release x64, retargeted to the installed toolset) ---
$sdkProjects = @(
    "pfc\pfc.vcxproj",
    "foobar2000\SDK\foobar2000_SDK.vcxproj",
    "foobar2000\foobar2000_component_client\foobar2000_component_client.vcxproj"
)
foreach ($proj in $sdkProjects) {
    & $msbuild (Join-Path $sdk $proj) /p:Configuration=Release /p:Platform=x64 `
        /p:PlatformToolset=v143 /m /v:minimal /nologo
    if ($LASTEXITCODE -ne 0) { throw "msbuild failed: $proj" }
}

# --- 4. Compile + link the shim ---
New-Item -ItemType Directory -Force $build | Out-Null

# Single-source the component version (ADR-0025): read the workspace version
# from root Cargo.toml - anchored to the [workspace.package] section so a
# member-crate or profile version can never match - and emit the header
# foo_lmv.cpp includes. The header lands in build/ (gitignored); never committed.
$cargoToml = Get-Content -Raw (Join-Path $repo "Cargo.toml")
if ($cargoToml -notmatch '\[workspace\.package\][^\[]*?\bversion\s*=\s*"([^"]+)"') {
    throw "could not parse [workspace.package] version from Cargo.toml"
}
$version = $Matches[1]
Set-Content -Path (Join-Path $build "foo_lmv_version.h") -Encoding ascii `
    -Value "#define FOO_LMV_VERSION `"$version`""
Write-Host "version: $version -> $build\foo_lmv_version.h"

$libs = @(
    (Join-Path $sdk "foobar2000\SDK\x64\Release\foobar2000_SDK.lib"),
    (Join-Path $sdk "foobar2000\foobar2000_component_client\x64\Release\foobar2000_component_client.lib"),
    (Join-Path $sdk "pfc\x64\Release\pfc.lib"),
    (Join-Path $sdk "foobar2000\shared\shared-x64.lib"),
    $coreLib,
    # lmv_core.lib's system deps (from rustc --print native-static-libs)
    # plus user32/gdi32 for the component's own window.
    "opengl32.lib", "kernel32.lib", "ntdll.lib", "userenv.lib",
    "ws2_32.lib", "dbghelp.lib", "user32.lib", "gdi32.lib", "advapi32.lib",
    "shell32.lib"
)
$libArgs = ($libs | ForEach-Object { "`"$_`"" }) -join " "
$cl = "cl /nologo /std:c++17 /EHsc /MD /O2 /W3 /DNDEBUG /DUNICODE /D_UNICODE " +
    "/I `"$sdk`" /I `"$sdk\foobar2000`" /I `"$repo\core\include`" /I `"$build`" " +
    "/Fo`"$build\\`" `"$root\foo_lmv.cpp`" " +
    "/link /DLL /OUT:`"$build\foo_lmv.dll`" $libArgs"
cmd /c "`"$vcvars`" >nul && $cl"
if ($LASTEXITCODE -ne 0) { throw "cl failed" }
Write-Host "built: $build\foo_lmv.dll"

# --- 5. Optional install into the foobar2000 v2 profile ---
if ($Install) {
    $dest = Join-Path $env:APPDATA "foobar2000-v2\user-components-x64\foo_lmv"
    New-Item -ItemType Directory -Force $dest | Out-Null
    Copy-Item (Join-Path $build "foo_lmv.dll") $dest -Force
    Write-Host "installed: $dest\foo_lmv.dll (restart foobar2000)"
}

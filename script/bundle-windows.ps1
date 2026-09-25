<#
.SYNOPSIS
    把 release 构建打成一个自带 libvips 的 Windows 目录包。

.DESCRIPTION
    Windows 比 macOS 简单得多：可执行文件的导入表里只记 DLL 的文件名（没有 macOS 那种
    绝对 install_name），而加载器搜索的第一站就是 exe 所在目录。所以「把 vips 的 DLL
    拷到 exe 旁边」这件事本身就完成了打包，不需要改写引用，也不需要重签名。

.PARAMETER VipsDir
    解压好的 libvips Windows 目录，例如 C:\vips\vips-dev-w64-web-8.18.6。

.PARAMETER Variant
    用于提示格式支持差异：web（默认，体积小、支持 AVIF、无相机 RAW）或 all。

.PARAMETER CreateZip
    同时生成可上传到 GitHub Releases 的带版本号 ZIP。

.EXAMPLE
    .\script\bundle-windows.ps1 -VipsDir C:\vips\vips-dev-w64-web-8.18.6
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$VipsDir,
    [ValidateSet('web', 'all')][string]$Variant = 'web',
    [switch]$CreateZip
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$outDir = Join-Path $root 'dist\lumen-frame'
$exeName = 'lumen-frame.exe'
$appIcon = Join-Path $root 'assets\app-icon\LumenFrame.ico'

function Say($message) { Write-Host "==> $message" -ForegroundColor Cyan }

if (-not (Test-Path (Join-Path $VipsDir 'bin'))) {
    throw "在 $VipsDir 下找不到 bin\ 目录。请指向解压后的 vips-dev-w64-* 目录。"
}
if (-not (Test-Path $appIcon)) {
    throw "找不到 Windows 应用图标：$appIcon"
}

# ------------------------------------------------------------------ 1. 构建

Say 'cargo build --release（会将 LumenFrame.ico 嵌入 exe）'
Push-Location $root
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build --release 失败（退出码 $LASTEXITCODE）" }
} finally { Pop-Location }

# ------------------------------------------------------------------ 2. 组装

Say "输出到 $outDir"
$distRoot = [IO.Path]::GetFullPath((Join-Path $root 'dist'))
$resolvedOutDir = [IO.Path]::GetFullPath($outDir)
if (-not $resolvedOutDir.StartsWith($distRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw "输出目录不在 dist 内：$resolvedOutDir"
}
if (Test-Path $distRoot) {
    if ((Get-Item -LiteralPath $distRoot -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "dist 目录是重解析点，拒绝递归删除：$distRoot"
    }
}
if (Test-Path $outDir) {
    if ((Get-Item -LiteralPath $outDir -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "输出目录是重解析点，拒绝递归删除：$outDir"
    }
    Remove-Item -LiteralPath $outDir -Recurse -Force
}
New-Item -ItemType Directory -Path $outDir | Out-Null

Copy-Item (Join-Path $root "target\release\$exeName") $outDir

Say '拷贝 libvips 及其依赖的 DLL'
Copy-Item (Join-Path $VipsDir 'bin\*.dll') $outDir

# vips 的可选格式模块是运行期按 VIPS_LIBDIR 找的，布局要和 libvips 的约定一致。
$moduleSource = Join-Path $VipsDir 'lib\vips-modules-8.18'
if (Test-Path $moduleSource) {
    $moduleTarget = Join-Path $outDir 'vips-modules-8.18'
    New-Item -ItemType Directory -Path $moduleTarget | Out-Null
    Copy-Item (Join-Path $moduleSource '*.dll') $moduleTarget
    # 程序会在 exe 同目录下找 vips-modules-8.18，找到就把 VIPS_LIBDIR 指过去。
    Say "拷贝 $(@(Get-ChildItem $moduleTarget).Count) 个 vips 模块"
} else {
    Say '没有 vips-modules 目录（该构建把格式全部内置了）'
}

# ------------------------------------------------------------------ 3. 校验

Say '校验：主程序、随附 DLL 与 vips 插件的导入都必须在输出目录里'

# dumpbin 来自 Visual Studio，objdump 来自 MinGW —— 用哪个取决于本机工具链。
$dumpbin = Get-Command dumpbin.exe -ErrorAction SilentlyContinue
$objdump = Get-Command objdump.exe -ErrorAction SilentlyContinue
if (-not $dumpbin -and -not $objdump) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (Test-Path -LiteralPath $vswhere) {
        $vsInstall = & $vswhere -latest -prerelease -products '*' -property installationPath
        if ($vsInstall) {
            $dumpbinPath = Get-ChildItem -LiteralPath (Join-Path $vsInstall 'VC\Tools\MSVC') -Filter dumpbin.exe -Recurse -File -ErrorAction SilentlyContinue |
                Where-Object { $_.FullName -match 'Hostx64\\x64\\dumpbin\.exe$' } |
                Sort-Object FullName -Descending |
                Select-Object -First 1 -ExpandProperty FullName
            if ($dumpbinPath) { $dumpbin = Get-Command $dumpbinPath }
        }
    }
}

# 系统 DLL 由 Windows 提供，其余必须随包分发。
$systemPrefixes = @('api-ms-', 'ext-ms-', 'kernel32', 'user32', 'gdi32', 'advapi32',
    'shell32', 'ole32', 'oleaut32', 'combase', 'comctl32', 'ws2_32', 'bcrypt', 'ntdll', 'crypt32',
    'dwmapi', 'dcomp', 'uxtheme', 'comdlg32', 'winmm', 'imm32', 'd3d', 'dxgi', 'opengl32',
    'msvcrt', 'vcruntime', 'ucrtbase', 'shlwapi', 'version', 'setupapi', 'cfgmgr32',
    'propsys', 'wintrust', 'userenv', 'powrprof', 'rpcrt4', 'secur32', 'dnsapi',
    'iphlpapi', 'netapi32', 'wtsapi32', 'mswsock', 'normaliz', 'pdh', 'psapi',
    'd2d1', 'windowscodecs', 'wldap32', 'dbghelp', 'ksuser', 'avrt', 'mfplat',
    'mfuuid', 'mfreadwrite', 'gdiplus', 'msimg32', 'usp10', 'dwrite', 'icuuc', 'uiautomationcore')

function Get-Imports([string]$path) {
    if ($dumpbin) {
        $output = & $dumpbin /dependents $path
        if ($LASTEXITCODE -ne 0) { throw "dumpbin 无法读取 $path（退出码 $LASTEXITCODE）" }
        return @($output |
            Select-String -Pattern '^\s+(\S+\.dll)\s*$' |
            ForEach-Object { $_.Matches[0].Groups[1].Value })
    }
    if ($objdump) {
        $output = & $objdump -p $path
        if ($LASTEXITCODE -ne 0) { throw "objdump 无法读取 $path（退出码 $LASTEXITCODE）" }
        return @($output |
            Select-String -Pattern 'DLL Name:\s*(\S+)' |
            ForEach-Object { $_.Matches[0].Groups[1].Value })
    }
    return @()
}

if (-not $dumpbin -and -not $objdump) {
    throw '找不到 dumpbin 或 objdump，无法校验发布包的 DLL 导入表'
} else {
    # vips 的格式插件也是运行期加载的 DLL；扫描整个输出树，才能发现它们缺失的依赖。
    $binaries = @(Get-ChildItem $outDir -Recurse -File | Where-Object {
        $_.Name -eq $exeName -or $_.Extension -eq '.dll'
    })
    $missing = @()

    foreach ($binary in $binaries) {
        foreach ($dll in (Get-Imports $binary.FullName | Sort-Object -Unique)) {
            $lowerName = $dll.ToLower()
            $isSystem = $systemPrefixes | Where-Object { $lowerName.StartsWith($_) } | Select-Object -First 1
            if ($isSystem) { continue }

            if (-not (Test-Path (Join-Path $outDir $dll))) {
                $missing += "$($binary.FullName.Substring($outDir.Length + 1)) -> $dll"
            }
        }
    }

    if ($missing.Count -gt 0) {
        $missing | Sort-Object -Unique | ForEach-Object { Write-Host "    缺少 $_" -ForegroundColor Red }
        throw '有 DLL 没有随包分发，换台机器会启动失败'
    }
    Say "  已校验 $($binaries.Count) 个 PE 文件，导入的 DLL 全部就位"
}

# ------------------------------------------------------------------ 4. 报告

$size = (Get-ChildItem $outDir -Recurse | Measure-Object -Property Length -Sum).Sum / 1MB
Say ('完成：{0}' -f $outDir)
Write-Host ('  体积：{0:N1} MB' -f $size)
Write-Host ('  文件：{0} 个' -f (Get-ChildItem $outDir -Recurse -File).Count)

if ($Variant -eq 'web') {
    Write-Host ''
    if (Test-Path (Join-Path $outDir 'libheif.dll')) {
        Write-Host '  本包包含 libheif；AVIF 已在 8.18.6 web 包验证，HEIC 仍需用真实样张验证。' -ForegroundColor Yellow
    } else {
        Write-Host '  本包未包含 libheif，不支持 HEIC/AVIF。' -ForegroundColor Yellow
    }
    if (-not (Test-Path (Join-Path $outDir 'libraw.dll'))) {
        Write-Host '  本包未包含 libraw，不支持相机 RAW。' -ForegroundColor Yellow
    }
}

if ($CreateZip) {
    $manifest = Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw
    if ($manifest -notmatch '(?m)^version\s*=\s*"([^"]+)"') {
        throw '无法从 Cargo.toml 读取应用版本'
    }
    $archive = Join-Path $distRoot ("Lumen-Frame-{0}-Windows-x86_64.zip" -f $Matches[1])
    if (Test-Path -LiteralPath $archive) {
        Remove-Item -LiteralPath $archive -Force
    }
    # self_update 默认从 ZIP 根目录读取 lumen-frame.exe，不能再包一层 lumen-frame/。
    Compress-Archive -Path (Join-Path $outDir '*') -DestinationPath $archive -CompressionLevel Optimal
    if (-not (Test-Path -LiteralPath $archive)) { throw "未生成发布包：$archive" }
    Say "发布包：$archive"
}

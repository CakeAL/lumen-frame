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
    用于提示格式支持差异：web（默认，体积小、无 HEIC/AVIF 与相机 RAW）或 all。

.EXAMPLE
    .\script\bundle-windows.ps1 -VipsDir C:\vips\vips-dev-w64-web-8.18.6
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$VipsDir,
    [ValidateSet('web', 'all')][string]$Variant = 'web'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$outDir = Join-Path $root 'dist\lumen-frame'
$exeName = 'lumen-frame.exe'

function Say($message) { Write-Host "==> $message" -ForegroundColor Cyan }

if (-not (Test-Path (Join-Path $VipsDir 'bin'))) {
    throw "在 $VipsDir 下找不到 bin\ 目录。请指向解压后的 vips-dev-w64-* 目录。"
}

# ------------------------------------------------------------------ 1. 构建

Say 'cargo build --release'
Push-Location $root
try { cargo build --release } finally { Pop-Location }

# ------------------------------------------------------------------ 2. 组装

Say "输出到 $outDir"
if (Test-Path $outDir) { Remove-Item -Recurse -Force $outDir }
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

# 系统 DLL 由 Windows 提供，其余必须随包分发。
$systemPrefixes = @('api-ms-', 'ext-ms-', 'kernel32', 'user32', 'gdi32', 'advapi32',
    'shell32', 'ole32', 'oleaut32', 'ws2_32', 'bcrypt', 'ntdll', 'crypt32',
    'dwmapi', 'uxtheme', 'comdlg32', 'winmm', 'imm32', 'd3d', 'dxgi', 'opengl32',
    'msvcrt', 'vcruntime', 'ucrtbase', 'shlwapi', 'version', 'setupapi', 'cfgmgr32',
    'propsys', 'wintrust', 'userenv', 'powrprof', 'rpcrt4', 'secur32', 'dnsapi',
    'iphlpapi', 'netapi32', 'wtsapi32', 'mswsock', 'normaliz', 'pdh', 'psapi',
    'd2d1', 'windowscodecs', 'wldap32', 'dbghelp', 'ksuser', 'avrt', 'mfplat',
    'mfuuid', 'mfreadwrite', 'gdiplus', 'msimg32', 'usp10', 'dwrite')

function Get-Imports([string]$path) {
    if ($dumpbin) {
        return @(& $dumpbin /dependents $path |
            Select-String -Pattern '^\s+(\S+\.dll)\s*$' |
            ForEach-Object { $_.Matches[0].Groups[1].Value })
    }
    if ($objdump) {
        return @(& $objdump -p $path |
            Select-String -Pattern 'DLL Name:\s*(\S+)' |
            ForEach-Object { $_.Matches[0].Groups[1].Value })
    }
    return @()
}

if (-not $dumpbin -and -not $objdump) {
    Write-Warning '找不到 dumpbin 或 objdump，跳过导入表校验（建议装 Visual Studio Build Tools 或 MinGW）。'
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
    Write-Host '  注意：web 变体不含 libheif / libraw，因此不支持 HEIC/AVIF 与相机 RAW。' -ForegroundColor Yellow
    Write-Host '        界面的文件选择器目前仍会接受这些扩展名，建议按变体调整支持列表。' -ForegroundColor Yellow
}

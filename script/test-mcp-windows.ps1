# 用 UTF-8 检查真实 stdio MCP；可选验证桌面修改，结束前恢复原水印。
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [switch]$ExerciseWorkspace,
    [ValidateRange(1, 300)][int]$TimeoutSeconds = 15
)
$ErrorActionPreference = 'Stop'
$info = [Diagnostics.ProcessStartInfo]::new((Resolve-Path -LiteralPath $Executable).Path, '--mcp-stdio')
$info.UseShellExecute = $false
$info.CreateNoWindow = $true
$info.RedirectStandardInput = $true
$info.RedirectStandardOutput = $true
$info.RedirectStandardError = $true
# 不依赖 Windows 代码页或调用方终端的编码；中文名称、模板和路径都需要 UTF-8。
$utf8 = [Text.UTF8Encoding]::new($false)
$info.StandardInputEncoding = $utf8
$info.StandardOutputEncoding = $utf8
$info.StandardErrorEncoding = $utf8
$process = [Diagnostics.Process]::new()
$process.StartInfo = $info
$script:requestId = 0
$script:transportFailed = $false
$original = $null
$restoreRevision = $null
$stderr = $null

function Invoke-McpRequest($Method, $Parameters) {
    $script:requestId++
    $payload = @{jsonrpc='2.0';id=$script:requestId;method=$Method;params=$Parameters} |
        ConvertTo-Json -Depth 100 -Compress
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $process.StandardInput.WriteLine($payload)
    while ($timer.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
        $line = $process.StandardOutput.ReadLineAsync()
        $remaining = [Math]::Max(1, [int]($TimeoutSeconds * 1000 - $timer.ElapsedMilliseconds))
        if (-not $line.Wait($remaining)) {
            $script:transportFailed = $true
            throw "MCP timeout: $Method $($Parameters.name); operation result unknown"
        }
        if ($null -eq $line.Result) {
            $script:transportFailed = $true
            throw 'MCP closed stdout; check stderr (including UTF-8 diagnostics)'
        }
        $reply = $line.Result | ConvertFrom-Json
        if ($reply.id -ne $script:requestId) { continue }
        if ($reply.error) { throw ($reply.error | ConvertTo-Json -Depth 20 -Compress) }
        if ($reply.result.isError) { throw ($reply.result.structuredContent | ConvertTo-Json -Depth 20 -Compress) }
        Write-Host "$Method $($Parameters.name): $($timer.ElapsedMilliseconds) ms"
        return $reply.result
    }
    throw "MCP timeout: $Method $($Parameters.name)"
}
function Invoke-McpTool($Name, $Arguments) {
    return (Invoke-McpRequest 'tools/call' @{name=$Name;arguments=$Arguments}).structuredContent
}
try {
    $null = $process.Start()
    $stderr = $process.StandardError.ReadToEndAsync()
    $null = Invoke-McpRequest 'initialize' @{
        protocolVersion='2025-03-26';capabilities=@{};clientInfo=@{name='windows-mcp-smoke';version='1'}
    }
    $process.StandardInput.WriteLine('{"jsonrpc":"2.0","method":"notifications/initialized"}')
    $tools = Invoke-McpRequest 'tools/list' @{}
    if (-not ($tools.tools | Where-Object name -eq 'load_preset')) { throw 'Missing load_preset tool' }
    $state = Invoke-McpTool 'get_state' @{}
    $null = Invoke-McpTool 'list_presets' @{}
    if ($ExerciseWorkspace) {
        $original = $state.data.current.watermark
        $revision = $state.revision
        foreach ($name in @('右侧参数卡_文字加框', 'colorwalk_背景色卡')) {
            $loaded = Invoke-McpTool 'load_preset' @{name=$name;all_photos=$false;expected_revision=$revision}
            $restoreRevision = $loaded.revision
            $state = Invoke-McpTool 'get_state' @{}
            if ($state.revision -ne $restoreRevision) { throw 'Workspace changed concurrently; restore skipped' }
            $revision = $state.revision
        }
        $count = @($state.data.current.watermark.text_groups).Count
        $added = Invoke-McpTool 'add_text_group' @{
            expected_revision=$revision
            group=@{
                text=@{template=@('下方文字框 📷');text_params=@(@{})}
                position='Bottom';direction='Horizontal';align='Center';time_format='%Y/%m/%d'
            }
        }
        $restoreRevision = $added.revision
        $state = Invoke-McpTool 'get_state' @{}
        if (@($state.data.current.watermark.text_groups).Count -ne $count + 1) { throw 'Text group was not added' }
    }
} finally {
    try {
        if ($null -ne $restoreRevision -and -not $script:transportFailed -and -not $process.HasExited) {
            $null = Invoke-McpTool 'set_watermark' @{
                watermark=$original;all_photos=$false;expected_revision=$restoreRevision
            }
            Write-Host 'Original watermark restored.'
        } elseif ($null -ne $restoreRevision) {
            Write-Warning 'Connection lost; operation result unknown. Original watermark could not be restored.'
        }
    } finally {
        if ($process.Id -and -not $process.HasExited) {
            $process.StandardInput.Close()
            if (-not $process.WaitForExit(2000)) { $process.Kill(); $process.WaitForExit() }
        }
        if ($stderr -and $stderr.Wait(2000) -and $stderr.Result) { Write-Host $stderr.Result }
        $process.Dispose()
    }
}
Write-Host 'Windows MCP checks passed.'

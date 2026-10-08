# 构建真实 ExitProcess 夹具；仅验证 CLI 完整退出状态，不授予产品能力。
$ErrorActionPreference = 'Stop'
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if ($LASTEXITCODE -ne 0 -or !$installation) { throw 'Existing MSVC required' }
$vcvars = Join-Path $installation 'VC\Auxiliary\Build\vcvars64.bat'
$output = Join-Path $env:RUNNER_TEMP 'diskgraph-windows-exit-fixtures'
New-Item -ItemType Directory -Path $output | Out-Null
$batch = Join-Path $output 'vcvars.cmd'
@('@echo off', "call `"$vcvars`" >nul", 'if errorlevel 1 exit /b %errorlevel%', 'set') | Set-Content -Encoding ascii $batch
$variables = & cmd.exe /d /c $batch
if ($LASTEXITCODE -ne 0) { throw 'MSVC environment failed' }
foreach ($line in $variables) {
    if ($line -match '^([^=]+)=(.*)$') {
        [Environment]::SetEnvironmentVariable($matches[1], $matches[2], 'Process')
    }
}
$source = (Resolve-Path 'crates/diskgraph-cli/tests/fixtures/windows_exit_status_fixture.c').Path
$receipts = @()
Push-Location $output
try {
    foreach ($bits in @('C0000000', 'C0000005')) {
        $binary = Join-Path $output "exit_$bits.exe"
        & cl.exe /nologo /utf-8 /W4 /WX /O2 "/DFIXTURE_EXIT_CODE=0x${bits}UL" "/Fe:$binary" $source
        if ($LASTEXITCODE -ne 0) { throw "Native exit fixture build failed: $bits" }
        $receipts += @{
            code = $bits
            path = $binary
            sha256 = (Get-FileHash -Algorithm SHA256 $binary).Hash.ToLowerInvariant()
            bytes = (Get-Item $binary).Length
        }
    }
} finally {
    Pop-Location
}
@{
    commit = (git rev-parse HEAD)
    source_sha256 = (Get-FileHash -Algorithm SHA256 $source).Hash.ToLowerInvariant()
    artifacts = $receipts
    status = 'compiled; direct and CLI execution remain required'
} | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 (Join-Path $output 'receipt.json')
# 所有产物构建成功后绑定绝对路径；实际测试独立验证两次原 32 位失败状态。
foreach ($receipt in $receipts) {
    "DISKGRAPH_WINDOWS_EXIT_$($receipt.code)_FIXTURE=$($receipt.path)" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
}

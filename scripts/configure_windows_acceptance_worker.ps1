# 从当前 checkout 的真实 Cargo binary 构造独占验收副本，不接受远程程序选择。
param([Parameter(Mandatory=$true)][string]$Artifacts, [Parameter(Mandatory=$true)][string]$OutputDir)
$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 使用 .NET Framework；只接受盘符绝对路径或完整 UNC 根。
# IsPathRooted 单独使用会接受 C:relative 与 \relative，不能替代原完整定位约束。
function Test-FullyQualifiedArtifactPath([string]$Path) {
    $root = [IO.Path]::GetPathRoot($Path)
    return $root -match '^(?:[A-Za-z]:[\\/]|[\\/]{2}[^\\/]+[\\/][^\\/]+)'
}
$rows = Get-Content -LiteralPath $Artifacts | ForEach-Object { $_ | ConvertFrom-Json }
$bins = @($rows | Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'diskgraph-scan-worker' -and $_.target.kind.Count -eq 1 -and $_.target.kind[0] -eq 'bin' -and $_.executable })
if ($bins.Count -ne 1) { throw 'One current Cargo worker binary is required' }
$source = [string]$bins[0].executable
$destination = Join-Path $OutputDir 'product-scan-worker.exe'
if (-not (Test-FullyQualifiedArtifactPath $source) -or -not (Test-FullyQualifiedArtifactPath $destination) -or $source -match "[\r\n]" -or $destination -match "[\r\n]") { throw 'Absolute clean artifact paths required' }
$info = Get-Item -LiteralPath $source
if ($info.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Cargo worker cannot be a reparse point' }
# 读取期间禁止其他写入和删除；副本不受后续 Cargo 重建覆盖。
$inputStream = [IO.File]::Open($source, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
try {
    if ($inputStream.Length -le 0 -or $inputStream.Length -gt 134217728) { throw 'Worker length outside admission budget' }
    $size = $inputStream.Length
    $outputStream = [IO.File]::Open($destination, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $inputStream.CopyTo($outputStream, 65536); $outputStream.Flush($true) } finally { $outputStream.Dispose() }
    $inputStream.Position = 0
    $hasher = [Security.Cryptography.SHA256]::Create()
    try { $digest = [BitConverter]::ToString($hasher.ComputeHash($inputStream)).Replace('-', '') } finally { $hasher.Dispose() }
    if ((Get-Item -LiteralPath $destination).Length -ne $size -or (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -ne $digest) { throw 'Exclusive worker snapshot differs from original artifact' }
} finally { $inputStream.Dispose() }
$sha = git rev-parse HEAD
if ($LASTEXITCODE -ne 0) { throw 'Missing current checkout identity' }
@{ checkout_sha = $sha; worker_path = $destination; cargo_artifact_path = $source; worker_sha256 = $digest; worker_bytes = $size; trust_source = 'current CI checkout Cargo binary artifact snapshot'; product_acceptance = 'pending' } | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $OutputDir 'product-worker-receipt.json')
foreach ($entry in @("DISKGRAPH_SCAN_WORKER_PATH=$destination", "DISKGRAPH_SCAN_WORKER_SHA256=$digest", "DISKGRAPH_SCAN_WORKER_BYTES=$size", "DISKGRAPH_WINDOWS_WORKER_QUALIFICATION=$destination")) {
    $entry | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
}

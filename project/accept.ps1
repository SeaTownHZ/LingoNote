# M2.5 验收静态闸
# 用法：pwsh -NoProfile -File project\accept.ps1
# 退出码：0 全过 / 1 有失败
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
$rows = New-Object System.Collections.Generic.List[object]

function Add-Row($name, $ok, $detail) {
  $rows.Add([pscustomobject]@{ 检查 = $name; 结果 = $(if ($ok) { 'PASS' } else { 'FAIL' }); 说明 = $detail })
}

$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# 1 前端类型检查 + 构建
Push-Location $root
$build = (npm run build 2>&1 | Out-String)
$buildOk = $LASTEXITCODE -eq 0
Pop-Location
$buildTail = (($build -split "`n") | Where-Object { $_ -match 'built in|error|✓' } | Select-Object -Last 3) -join ' / '
Add-Row '前端构建 npm run build' $buildOk $(if ($buildOk) { $buildTail } else { $buildTail })

# 2 Rust 测试
Push-Location (Join-Path $root 'src-tauri')
$test = (cargo test --release --offline 2>&1 | Out-String)
$testOk = $LASTEXITCODE -eq 0
Pop-Location
$passed = 0
foreach ($m in [regex]::Matches($test, 'test result: ok\. (\d+) passed')) { $passed += [int]$m.Groups[1].Value }
Add-Row 'Rust 测试 cargo test' ($testOk -and $passed -ge 20) "通过 $passed 个（要求 >= 20）"

# 3 生产构建（必须走 Tauri CLI；裸 cargo build --release 会带 devUrl，产物打不开）
Push-Location $root
$release = (npm run tauri build -- --no-bundle 2>&1 | Out-String)
$releaseOk = $LASTEXITCODE -eq 0
Pop-Location
$exe = Join-Path $root 'src-tauri\target\release\lingonote.exe'
$exeOk = Test-Path $exe
$releaseDetail = if ($releaseOk -and $exeOk) {
  "lingonote.exe {0:N1} MB（Tauri CLI 生产构建）" -f ((Get-Item $exe).Length / 1MB)
} elseif ($exeOk) {
  '构建失败，但存在旧产物'
} else {
  '产物缺失'
}
Add-Row '生产构建 tauri build' ($releaseOk -and $exeOk) $releaseDetail

# 4 配置契约静态检查：Rust 与 TS 都必须出现新增字段
$pure = Get-Content (Join-Path $root 'src-tauri\src\pure.rs') -Raw
$lib = Get-Content (Join-Path $root 'src-tauri\src\lib.rs') -Raw
$settings = Get-Content (Join-Path $root 'src\settings.ts') -Raw
$main = Get-Content (Join-Path $root 'src\main.ts') -Raw
$css = Get-Content (Join-Path $root 'src\styles.css') -Raw
$checks = [ordered]@{
  'pure.rs: opacity'            = $pure -match 'pub opacity'
  'pure.rs: WindowConfig'       = $pure -match 'pub struct WindowConfig'
  'pure.rs: anchor_position'    = $pure -match 'fn anchor_position'
  'lib.rs: position_mode'       = $lib -match 'position_mode'
  'settings.ts: window'         = $settings -match 'window:'
  'settings.ts: opacity'        = $settings -match 'opacity'
  'main.ts: --note-opacity'     = $main -match '--note-opacity'
  'styles.css: var(--note-opacity' = $css -match 'var\(--note-opacity'
}
$bad = @($checks.GetEnumerator() | Where-Object { -not $_.Value } | ForEach-Object { $_.Key })
Add-Row '配置契约静态检查' ($bad.Count -eq 0) $(if ($bad.Count) { "缺：$($bad -join ', ')" } else { '8 项关键字齐全' })

# 5 工作区干净度提醒（不阻断）
Push-Location $root
$dirty = (git status --short 2>$null | Measure-Object).Count
Pop-Location
Add-Row '工作区待提交' ($dirty -ge 0) "$dirty 个改动（验收后统一提交）"

$rows | Format-Table -AutoSize
$fails = @($rows | Where-Object { $_.结果 -eq 'FAIL' }).Count
"`n合计：PASS $($rows.Count - $fails) / FAIL $fails"
exit $(if ($fails -eq 0) { 0 } else { 1 })

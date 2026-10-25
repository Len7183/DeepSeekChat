# build-installer.ps1 —— 一键构建 NSIS 安装包（FR-15）
# 产物：dist\DeepSeekChat-v<版本>-setup.exe
# 特性：全局/当前用户二选一、可改路径、finish 页勾选（桌面快捷方式默认创建、
#       开始菜单默认不创建、开机自启默认不）、卸载默认删数据可勾保留、WebView2 引导
# 体积上限：8MB（超限构建失败）
[CmdletBinding()]
param(
    [switch]$SkipFrontend
)
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path

Write-Host "== DeepSeekChat NSIS 安装包构建 ==" -ForegroundColor Cyan

# ---- 0. 工具链检查 ----
# rustup 默认安装路径可能尚未进入当前进程 PATH
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
    if (Test-Path (Join-Path $cargoBin 'cargo.exe')) {
        $env:PATH = "$cargoBin;$env:PATH"
    }
}
foreach ($tool in @('node', 'npm')) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "缺少工具链: $tool（请先安装 Node LTS）"
    }
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "缺少工具链: cargo（请先安装 Rust stable）"
}

# ---- 1. 版本号 ----
$conf = Get-Content "$Root\src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json
$version = $conf.version
Write-Host "版本: v$version"

# ---- 2. 前端构建 ----
if (-not $SkipFrontend) {
    Write-Host "== 前端构建 ==" -ForegroundColor Cyan
    Push-Location "$Root\settings-ui"
    try {
        # 用 cmd 包裹避免 PS5.1 把 npm 的 stderr 警告升级为终止错误；
        # 退出码在管道后仍取自 cmd（ForEach-Object 不覆盖 $LASTEXITCODE）
        $npmOut = cmd /c "npm install --no-audit --no-fund 2>&1"
        $npmCode = $LASTEXITCODE
        $npmOut | Where-Object { $_ } | ForEach-Object { Write-Host "  $_" }
        if ($npmCode -ne 0) { throw "npm install 失败（exit=$npmCode，见上方输出）" }
        npm run build
        if ($LASTEXITCODE -ne 0) { throw "前端构建失败" }
    }
    finally {
        Pop-Location
    }
}

# ---- 3. 素材检查 ----
$bootstrapper = "$Root\src-tauri\resources\MicrosoftEdgeWebview2Setup.exe"
if (-not (Test-Path $bootstrapper)) {
    Write-Host "下载微软官方 WebView2 Bootstrapper..." -ForegroundColor Yellow
    New-Item -ItemType Directory -Force "$Root\src-tauri\resources" | Out-Null
    Invoke-WebRequest -Uri 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' -OutFile $bootstrapper
}

# ---- 4. NSIS 模板定制（与 tauri-cli 版本强相关，脚本自动拉取并打补丁） ----
$template = "$Root\src-tauri\nsis\installer.nsi"
if (-not (Test-Path $template)) {
    Write-Host "下载官方 NSIS 模板（tauri-cli-v2.12.0）..." -ForegroundColor Yellow
    New-Item -ItemType Directory -Force "$Root\src-tauri\nsis" | Out-Null
    Invoke-WebRequest -Uri 'https://raw.githubusercontent.com/tauri-apps/tauri/tauri-cli-v2.12.0/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi' -OutFile $template
    node "$Root\scripts\patch-nsis-template.mjs" $template
    if ($LASTEXITCODE -ne 0) { throw "NSIS 模板补丁失败" }
}

# ---- 5. Tauri 构建（NSIS 安装包；tauri-cli 自动下载 NSIS 工具链） ----
# 必须在仓库根目录执行（tauri-cli 由此定位 src-tauri\tauri.conf.json），
# cli 二进制取自 settings-ui 的 npm 依赖
Write-Host "== tauri build（nsis）==" -ForegroundColor Cyan
$tauriCli = "$Root\settings-ui\node_modules\.bin\tauri.CMD"
if (-not (Test-Path $tauriCli)) { throw "未找到 @tauri-apps/cli，请先在 settings-ui 执行 npm install" }
# 与绿色版使用相同的路径重映射，避免安装包内主程序暴露构建机路径。
# 同 build-portable.ps1：已有编码值时不再另拆 RUSTFLAGS（防劈碎含空格参数）
$previousFlags = $env:CARGO_ENCODED_RUSTFLAGS
$flags = @(if ($previousFlags) { $previousFlags -split [char]31 } else { @($env:RUSTFLAGS) | Where-Object { $_ } })
foreach ($prefix in @($Root, $env:USERPROFILE, $env:CARGO_HOME, $env:RUSTUP_HOME) | Where-Object { $_ } | Select-Object -Unique) {
    $flags += "--remap-path-prefix=$prefix=/build"
    $flags += "--remap-path-prefix=$($prefix.Replace('\', '/'))=/build"
}
$env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]31
Push-Location $Root
try {
    & $tauriCli build --bundles nsis
    if ($LASTEXITCODE -ne 0) { throw "Tauri 构建失败" }
} finally {
    Pop-Location
    $env:CARGO_ENCODED_RUSTFLAGS = $previousFlags
}

# ---- 6. 产物与体积校验（先验证再入 dist，防超限产物被误分发） ----
node "$Root\scripts\check-release.mjs" "$Root\src-tauri\target\release\deepseekchat.exe"
if ($LASTEXITCODE -ne 0) { throw "发布隐私检查失败" }
$bundle = Get-ChildItem "$Root\src-tauri\target\release\bundle\nsis" -Filter "*.exe" |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $bundle) { throw "未找到 NSIS 产物" }
$mb = [math]::Round($bundle.Length / 1MB, 2)
if ($mb -gt 8) {
    throw "NSIS 安装包体积 $mb MB 超过 8MB 上限（未复制进 dist）"
}
New-Item -ItemType Directory -Force "$Root\dist" | Out-Null
$out = "$Root\dist\DeepSeekChat-v$version-setup.exe"
Copy-Item $bundle.FullName $out -Force
Write-Host ("产物: {0}（{1} MB）" -f $out, $mb) -ForegroundColor Green
Write-Host "== 构建成功，体积达标 ==" -ForegroundColor Green

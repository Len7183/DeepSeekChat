# build-portable.ps1 —— 一键构建绿色单 exe（FR-16）
# 产物：dist\DeepSeekChat-v<版本>-portable.exe（内嵌全部前端资源与 WebView2 Bootstrapper）
# 体积上限：15MB（超限构建失败）
[CmdletBinding()]
param(
    [switch]$SkipFrontend
)
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path

Write-Host "== DeepSeekChat 绿色版构建 ==" -ForegroundColor Cyan

# ---- 0. 工具链检查 ----
# rustup 默认安装路径可能尚未进入当前进程 PATH
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
    if (Test-Path (Join-Path $cargoBin 'cargo.exe')) {
        $env:PATH = "$cargoBin;$env:PATH"
    }
}
foreach ($tool in @('cargo', 'node', 'npm')) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "缺少工具链: $tool（请先安装 Rust stable 与 Node LTS）"
    }
}

# ---- 1. 版本号 ----
$conf = Get-Content "$Root\src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json
$version = $conf.version
Write-Host "版本: v$version"

# ---- 2. 前端构建（设置窗口/错误页/托盘菜单） ----
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

# ---- 3. 素材检查（WebView2 Bootstrapper 必须内嵌） ----
$bootstrapper = "$Root\src-tauri\resources\MicrosoftEdgeWebview2Setup.exe"
if (-not (Test-Path $bootstrapper)) {
    Write-Host "下载微软官方 WebView2 Bootstrapper..." -ForegroundColor Yellow
    New-Item -ItemType Directory -Force "$Root\src-tauri\resources" | Out-Null
    $tmp = "$bootstrapper.download"
    # 先下载到临时名：中断的半截文件绝不能占用正式名（下次 Test-Path 会误判已就绪）
    Invoke-WebRequest -Uri 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' -OutFile $tmp
    # 完整性校验：官方 Bootstrapper 是 PE 文件（MZ 头）且体积在已知合理区间
    # （约 1.5~3MB）；fwlink 重定向被劫持/截断时在此拦截，而不是把坏文件内嵌进发布 exe
    $bytes = [System.IO.File]::ReadAllBytes($tmp)
    if ($bytes.Length -lt 1MB -or $bytes.Length -gt 3MB -or $bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) {
        Remove-Item $tmp -Force -ErrorAction SilentlyContinue
        throw "Bootstrapper 下载异常（大小 $($bytes.Length) 字节或非 PE 格式），已丢弃；请检查网络后重试"
    }
    Move-Item $tmp $bootstrapper -Force
}
$bootMb = [math]::Round((Get-Item $bootstrapper).Length / 1MB, 2)
Write-Host "Bootstrapper: $bootMb MB（内嵌）"

# ---- 4. Rust 构建 ----
# 必须启用 tauri/custom-protocol：否则本地页（设置/托盘菜单/错误页）仍路由到
# devUrl（localhost:5173），发布产物里全部白屏（dev cfg 由此 feature 决定）
Write-Host "== cargo build --release（custom-protocol）==" -ForegroundColor Cyan
# 编码参数保留带空格的路径，并在结束后恢复调用者的编译环境。
# 注意：只在 CARGO_ENCODED_RUSTFLAGS 为空时才从 RUSTFLAGS 起步——已有编码
# 值时另拆 RUSTFLAGS 会把含空格的参数劈碎（编码分隔符 0x1F 正是为此存在）
$previousFlags = $env:CARGO_ENCODED_RUSTFLAGS
$flags = @(if ($previousFlags) { $previousFlags -split [char]31 } else { @($env:RUSTFLAGS) | Where-Object { $_ } })
foreach ($prefix in @($Root, $env:USERPROFILE, $env:CARGO_HOME, $env:RUSTUP_HOME) | Where-Object { $_ } | Select-Object -Unique) {
    $flags += "--remap-path-prefix=$prefix=/build"
    $flags += "--remap-path-prefix=$($prefix.Replace('\', '/'))=/build"
}
$env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]31
try {
    cargo build --release --manifest-path "$Root\src-tauri\Cargo.toml" --features tauri/custom-protocol
    if ($LASTEXITCODE -ne 0) { throw "Rust 构建失败" }
}
finally {
    $env:CARGO_ENCODED_RUSTFLAGS = $previousFlags
}

# ---- 5. 产物与体积校验（先验证再入 dist，防超限产物被误分发） ----
$src = "$Root\src-tauri\target\release\deepseekchat.exe"
node "$Root\scripts\check-release.mjs" $src
if ($LASTEXITCODE -ne 0) { throw "发布隐私检查失败" }
$mb = [math]::Round((Get-Item $src).Length / 1MB, 2)
if ($mb -gt 15) {
    throw "绿色 exe 体积 $mb MB 超过 15MB 上限（未复制进 dist）"
}
New-Item -ItemType Directory -Force "$Root\dist" | Out-Null
$out = "$Root\dist\DeepSeekChat-v$version-portable.exe"
Copy-Item $src $out -Force
Write-Host ("产物: {0}（{1} MB）" -f $out, $mb) -ForegroundColor Green
Write-Host "== 构建成功，体积达标 ==" -ForegroundColor Green

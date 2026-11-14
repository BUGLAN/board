# Board 命令行安装脚本（Windows；macOS、Linux 用 install.sh）。PRD §9「安装与升级」。
#
# 服务端在 <服务端>/install.ps1 提供这个文件，下面三个 @…@ 由服务端换成它自己的版本号、地址和发版附件的地址：
#
#   irm <服务端>/install.ps1 | iex
#   & ([scriptblock]::Create((irm '<服务端>/install.ps1'))) -Token <令牌> -Name <机器名> -Workspace '<工作空间>' [-Dir '<项目>/<白板>=<路径>'] [-Service]
#
# 依次做：
#   1. 下载发版附件里的 board-windows-x64.zip 和校验文件 board-sha256.txt，SHA-256 对不上就中止（不动现有的 board）
#   2. 装到 %LOCALAPPDATA%\Board\bin（环境变量 BOARD_INSTALL_DIR 可改）；不在用户 PATH 里时写进去，只写一次
#   3. 带 -Token：board login --url <服务端> --token <令牌>
#   4. 带 -Name / -Workspace / -Dir：带 -Service 时装成开机自动运行（board host --install），否则给出前台运行的命令
#
# 环境变量 BOARD_DOWNLOAD 换下载地址（镜像，或本机目录，测试用）。
param(
  [string]$Token = '',
  [string]$Name = '',
  [string]$Workspace = '',
  [string[]]$Dir = @(),
  [switch]$Service
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$BoardVersion = '@BOARD_VERSION@'
$BoardServer = '@BOARD_SERVER@'
$BoardDownload = if ($env:BOARD_DOWNLOAD) { $env:BOARD_DOWNLOAD } else { '@BOARD_DOWNLOAD@' }

function Fail([string]$msg) {
  Write-Host "✗ $msg" -ForegroundColor Red
  throw $msg
}

if (($BoardVersion + $BoardServer + $BoardDownload) -like '*@BOARD_*') {
  Fail '这份脚本没有经过服务端：用 irm <服务端>/install.ps1 | iex 安装'
}
$arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($arch -ne 'AMD64') {
  Fail "发版附件里只有 Windows x64 的 board（这台是 $arch）：用 cargo install 从源码装"
}

# 取一个附件到本地文件：网址用 HTTP，其他当成本机目录
function Get-Asset([string]$file, [string]$out) {
  if ($BoardDownload -match '^https?://') {
    try { Invoke-WebRequest -UseBasicParsing -Uri "$BoardDownload/$file" -OutFile $out }
    catch { Fail "下载 $BoardDownload/$file 失败：$($_.Exception.Message)" }
  } else {
    $src = Join-Path $BoardDownload $file
    if (-not (Test-Path -LiteralPath $src)) { Fail "读不到 $src" }
    Copy-Item -LiteralPath $src -Destination $out
  }
}

$archive = 'board-windows-x64.zip'
$dir = if ($env:BOARD_INSTALL_DIR) { $env:BOARD_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Board\bin' }
Write-Host "→ 安装 board $BoardVersion（windows-x64）到 $dir"

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("board-install-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $tmp | Out-Null
try {
  Get-Asset 'board-sha256.txt' (Join-Path $tmp 'board-sha256.txt')
  Get-Asset $archive (Join-Path $tmp $archive)
  $want = $null
  foreach ($line in Get-Content -LiteralPath (Join-Path $tmp 'board-sha256.txt')) {
    $parts = $line.Trim() -split '\s+', 2
    if ($parts.Count -eq 2 -and $parts[1].TrimStart('*') -eq $archive) { $want = $parts[0].ToLower(); break }
  }
  if (-not $want) { Fail "校验文件里没有 $archive，没有安装" }
  $got = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $tmp $archive)).Hash.ToLower()
  if ($got -ne $want) {
    Fail "$archive 的 SHA-256 对不上（应为 $want，下载到的是 $got）：下载不完整或被改过，没有安装，现有的 board 没动"
  }

  $x = Join-Path $tmp 'x'
  Expand-Archive -LiteralPath (Join-Path $tmp $archive) -DestinationPath $x -Force
  $new = Join-Path $x 'board.exe'
  if (-not (Test-Path -LiteralPath $new)) { Fail '压缩包里没有 board.exe' }
  $v = (& $new --version) 2>$null
  if ($v -ne "board $BoardVersion") { Fail "下载到的 board 版本不对（$v，应为 $BoardVersion），没有安装" }

  New-Item -ItemType Directory -Force $dir | Out-Null
  $bin = Join-Path $dir 'board.exe'
  # 正在运行的 board.exe 不能覆盖、但能改名：先挪开，再放新的
  if (Test-Path -LiteralPath $bin) {
    $old = Join-Path $dir 'board.old.exe'
    Remove-Item -LiteralPath $old -Force -ErrorAction SilentlyContinue
    Move-Item -LiteralPath $bin -Destination $old -Force
  }
  Move-Item -LiteralPath $new -Destination $bin -Force
} finally {
  Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Host "✓ 已安装 $(& $bin --version)：$bin" -ForegroundColor Green

# 用户 PATH（和桌面端「Agent 接入」那条命令一样，只写一次）
$p = [Environment]::GetEnvironmentVariable('Path', 'User')
$parts = @($p -split ';' | Where-Object { $_ })
if ($parts -notcontains $dir) {
  [Environment]::SetEnvironmentVariable('Path', (($parts + $dir) -join ';'), 'User')
  Write-Host "  已把 $dir 写进用户 PATH（新开终端生效）"
}
if (-not (($env:Path -split ';') -contains $dir)) { $env:Path = "$dir;$env:Path" }

if ($Token) {
  & $bin login --url $BoardServer --token $Token
  if ($LASTEXITCODE -ne 0) { Fail "登录 $BoardServer 失败：令牌在界面「设置 → Agent 接入」新建" }
}

if ($Name -or $Workspace -or $Dir.Count) {
  $hostArgs = @('host')
  if ($Service) { $hostArgs += '--install' }
  if ($Name) { $hostArgs += @('--name', $Name) }
  if ($Workspace) { $hostArgs += @('--workspace', $Workspace) }
  foreach ($d in $Dir) { $hostArgs += @('--dir', $d) }
  if ($Service) {
    Write-Host '→ 装成开机自动运行 board host'
    & $bin @hostArgs
    if ($LASTEXITCODE -ne 0) { Fail 'board host --install 失败' }
  } else {
    Write-Host ''
    Write-Host '在这台机器上常驻运行（Ctrl+C 退出）：'
    Write-Host ('  board ' + (($hostArgs | ForEach-Object { "'" + ($_ -replace "'", "''") + "'" }) -join ' '))
  }
} elseif (-not $Token) {
  Write-Host ''
  Write-Host "下一步：在界面「设置 → Agent 接入」新建令牌，然后 board login --url $BoardServer --token <令牌>"
}

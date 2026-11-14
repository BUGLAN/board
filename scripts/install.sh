#!/bin/sh
# Board 命令行安装脚本（macOS、Linux；Windows 用 install.ps1）。PRD §9「安装与升级」。
#
# 服务端在 <服务端>/install.sh 提供这个文件，下面三个 @…@ 由服务端换成它自己的版本号、地址和发版附件的地址：
#
#   curl -fsSL <服务端>/install.sh | sh
#   curl -fsSL <服务端>/install.sh | sh -s -- --token <令牌> --name <机器名> --workspace <工作空间> [--dir <项目>/<白板>=<路径>] [--service]
#
# 依次做：
#   1. 按平台下载发版附件里的 board-<平台>.tar.gz 和校验文件 board-sha256.txt，SHA-256 对不上就中止（不动现有的 board）
#   2. 装到 ~/.local/bin（环境变量 BOARD_INSTALL_DIR 可改）；这个目录不在 PATH 里时写进 shell 配置，只追加一次
#   3. 带 --token：board login --url <服务端> --token <令牌>
#   4. 带 --name / --workspace / --dir：带 --service 时装成系统服务、开机自动接任务（board host --install），否则给出前台运行的命令
#
# 环境变量 BOARD_DOWNLOAD 换下载地址（镜像，或本机目录，测试用）。
set -eu

BOARD_VERSION='@BOARD_VERSION@'
BOARD_SERVER='@BOARD_SERVER@'
BOARD_DOWNLOAD="${BOARD_DOWNLOAD:-@BOARD_DOWNLOAD@}"

say() { printf '%s\n' "$*"; }
die() {
  printf '✗ %s\n' "$*" >&2
  exit 1
}

# 发版附件里的平台名：<uname -s> <uname -m> → darwin-arm64、darwin-x64、linux-x64、linux-arm64；不支持时返回 1
board_platform() {
  case "$1" in
    Darwin) os=darwin ;;
    Linux) os=linux ;;
    *) return 1 ;;
  esac
  case "$2" in
    x86_64 | amd64) arch=x64 ;;
    arm64 | aarch64 | armv8*) arch=arm64 ;;
    *) return 1 ;;
  esac
  printf '%s-%s\n' "$os" "$arch"
}

# 本机的平台。Apple Silicon 上用 Rosetta 跑的终端 uname -m 是 x86_64，按真实的芯片装 arm64
host_platform() {
  s=$(uname -s)
  m=$(uname -m)
  if [ "$s" = Darwin ] && [ "$m" = x86_64 ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then
    m=arm64
  fi
  board_platform "$s" "$m"
}

# 取一个附件到本地文件：网址用 curl / wget，其他当成本机目录
fetch() {
  case "$BOARD_DOWNLOAD" in
    http://* | https://*)
      if command -v curl >/dev/null 2>&1; then
        curl -fsSL --retry 2 -o "$2" "$BOARD_DOWNLOAD/$1" || die "下载 $BOARD_DOWNLOAD/$1 失败"
      elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$2" "$BOARD_DOWNLOAD/$1" || die "下载 $BOARD_DOWNLOAD/$1 失败"
      else
        die "要用 curl 或 wget 下载，两个都没有"
      fi
      ;;
    *) cp "$BOARD_DOWNLOAD/$1" "$2" 2>/dev/null || die "读不到 $BOARD_DOWNLOAD/$1" ;;
  esac
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d ' ' -f 1
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "$1" | sed 's/.*= *//'
  else
    die "要用 sha256sum、shasum 或 openssl 校验下载的文件，一个都没有"
  fi
}

# 校验文件里某个文件的 SHA-256（认 "<值>  <名>" 和 "<值> *<名>"）
sum_in() {
  awk -v f="$2" '{ n = $2; sub(/^\*/, "", n); if (n == f) { print tolower($1); exit } }' "$1"
}

# 装好的目录不在 PATH 里：写进 shell 配置（和桌面端「Agent 接入」那条命令一样，只追加一次）
ensure_path() {
  case ":$PATH:" in
    *":$1:"*) return 0 ;;
  esac
  case "${SHELL:-}" in
    */zsh) rc="$HOME/.zshrc" ;;
    */bash) rc="$HOME/.bashrc" ;;
    *) rc="$HOME/.profile" ;;
  esac
  line="export PATH=\"$1:\$PATH\""
  touch "$rc"
  if grep -qxF "$line" "$rc" 2>/dev/null; then
    say "  $1 已经写在 $rc 里（新开终端生效）"
  else
    printf '%s\n' "$line" >>"$rc"
    say "  已把 $1 写进 $rc 的 PATH（新开终端或 source $rc 后生效）"
  fi
}

# ---------------------------------------------------------------- 参数

token=''
name=''
workspace=''
dirs=''
service=0
while [ $# -gt 0 ]; do
  case "$1" in
    --token) token=${2:?--token 需要一个值}; shift 2 ;;
    --name) name=${2:?--name 需要一个值}; shift 2 ;;
    --workspace)
      workspace=${2:?--workspace 需要一个值，每块白板的目录是 <工作空间>/<项目>/<白板>}
      shift 2
      ;;
    --dir)
      d=${2:?--dir 需要一个值，写成 <项目>/<白板>=<路径>}
      dirs="$dirs$d
"
      shift 2
      ;;
    --service) service=1; shift ;;
    --platform-of)
      # 测试用：只算平台名
      board_platform "${2:-}" "${3:-}" || die "不支持的平台：${2:-} ${3:-}"
      exit 0
      ;;
    -h | --help)
      sed -n '2,15p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) die "未知参数 $1（可用：--token、--name、--workspace、--dir、--service）" ;;
  esac
done

case "$BOARD_VERSION$BOARD_SERVER$BOARD_DOWNLOAD" in
  *@BOARD_*) die "这份脚本没有经过服务端：用 curl -fsSL <服务端>/install.sh | sh 安装" ;;
esac

# ---------------------------------------------------------------- 下载、校验、安装

platform=$(host_platform) || die "发版附件里没有这个平台（$(uname -s) $(uname -m)）的 board：用 cargo install 从源码装"
archive="board-$platform.tar.gz"
dir="${BOARD_INSTALL_DIR:-$HOME/.local/bin}"
say "→ 安装 board $BOARD_VERSION（$platform）到 $dir"

tmp=$(mktemp -d 2>/dev/null || mktemp -d -t board)
trap 'rm -rf "$tmp"' EXIT INT TERM
fetch board-sha256.txt "$tmp/board-sha256.txt"
fetch "$archive" "$tmp/$archive"
want=$(sum_in "$tmp/board-sha256.txt" "$archive")
[ -n "$want" ] || die "校验文件里没有 $archive，没有安装"
got=$(sha256_of "$tmp/$archive")
[ "$got" = "$want" ] || die "$archive 的 SHA-256 对不上（应为 $want，下载到的是 $got）：下载不完整或被改过，没有安装，现有的 board 没动"

mkdir -p "$tmp/x"
tar -xzf "$tmp/$archive" -C "$tmp/x" || die "解压 $archive 失败"
[ -f "$tmp/x/board" ] || die "压缩包里没有 board"
chmod 755 "$tmp/x/board"
v=$("$tmp/x/board" --version 2>/dev/null || true)
[ "$v" = "board $BOARD_VERSION" ] || die "下载到的 board 版本不对（${v:-运行不了}，应为 $BOARD_VERSION），没有安装"

mkdir -p "$dir" || die "建不了目录 $dir"
cp "$tmp/x/board" "$dir/.board.new.$$" || die "写不了 $dir：换一个目录（BOARD_INSTALL_DIR=<目录>）或用有权限的用户运行"
chmod 755 "$dir/.board.new.$$"
mv -f "$dir/.board.new.$$" "$dir/board"
bin="$dir/board"
say "✓ 已安装 $("$bin" --version)：$bin"
ensure_path "$dir"

# ---------------------------------------------------------------- 登录、接任务

if [ -n "$token" ]; then
  "$bin" login --url "$BOARD_SERVER" --token "$token" || die "登录 $BOARD_SERVER 失败：令牌在界面「设置 → Agent 接入」新建"
fi

if [ -n "$name" ] || [ -n "$workspace" ] || [ -n "$dirs" ]; then
  set -- host
  [ "$service" = 1 ] && set -- host --install
  [ -n "$name" ] && set -- "$@" --name "$name"
  [ -n "$workspace" ] && set -- "$@" --workspace "$workspace"
  while IFS= read -r d; do
    [ -n "$d" ] && set -- "$@" --dir "$d"
  done <<EOF
$dirs
EOF
  if [ "$service" = 1 ]; then
    say "→ 装成系统服务：开机自动运行 board host"
    "$bin" "$@" </dev/null || die "board host --install 失败"
  else
    say ""
    say "在这台机器上常驻运行（Ctrl+C 退出）："
    printf '  board'
    for a in "$@"; do printf " '%s'" "$a"; done
    printf '\n'
  fi
elif [ -z "$token" ]; then
  say ""
  say "下一步：在界面「设置 → Agent 接入」新建令牌，然后 board login --url $BOARD_SERVER --token <令牌>"
fi

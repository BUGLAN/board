#!/bin/sh
# Board 心跳：领取卡片后在后台每 10 秒执行一次 `board beat <ID>`，表明 agent 还在工作。
#
#   sh heartbeat.sh start <ID>   先同步发一次心跳（失败直接报错退出），再在后台循环
#   sh heartbeat.sh stop  <ID>   停掉后台循环，发送 final（正常退出）
#
# stop：先通知后台循环停下，等它真正退出（正在执行的那次 board beat 会先发完），再发 final，
# 保证 final 是最后一条心跳；否则晚到的普通心跳会在 final 之后到达服务端。
#
# 后台循环在这些情况下自己停下，不发 final：
#   - agent 进程不在了（崩溃、被关掉）→ 服务端连续 5 次收不到心跳，判定异常退出，卡片回到领取起点
#   - board beat 被拒绝（卡片已经不归你、令牌失效等）；连不上服务端（UNREACHABLE）时继续重试
#
# agent 进程：从本脚本往上找第一个不是 shell 的祖先进程（Claude Code、Codex 等）。
# 可以用 BOARD_HEARTBEAT_PID 指定。需要 BOARD、BOARD_AS（以及 BOARD_URL、BOARD_TOKEN）已经设置。

set -u
INTERVAL=${BOARD_HEARTBEAT_INTERVAL:-10} # 只给测试用；正常使用不要改

die() { echo "heartbeat: $*" >&2; exit 1; }

pidfile() {
    dir="${TMPDIR:-/tmp}"
    dir="${dir%/}/board-heartbeat"
    mkdir -p "$dir" || die "无法创建 $dir"
    key=$(printf '%s-%s-%s' "${BOARD:-}" "$1" "${BOARD_AS:-}" | tr -c 'A-Za-z0-9._-' '_')
    echo "$dir/$key.pid"
}

# 第一个不是 shell 的祖先进程
agent_pid() {
    if [ -n "${BOARD_HEARTBEAT_PID:-}" ]; then
        echo "$BOARD_HEARTBEAT_PID"
        return
    fi
    pid=$PPID
    while [ -n "$pid" ] && [ "$pid" -gt 1 ]; do
        comm=$(ps -o comm= -p "$pid" 2>/dev/null) || break
        comm=$(basename -- "${comm#-}")
        case "$comm" in
            sh | bash | zsh | dash | ksh | mksh | fish | env | nohup | login | timeout | sudo) ;;
            *) echo "$pid"; return ;;
        esac
        pid=$(ps -o ppid= -p "$pid" 2>/dev/null | tr -d ' ')
    done
}

# 发一次心跳；成功返回 0，被拒绝返回 1（输出错误码），连不上返回 2
beat_once() {
    out=$(board beat "$1" --json 2>&1) && return 0
    code=$(printf '%s' "$out" | sed -n 's/.*"code": *"\([A-Z_]*\)".*/\1/p' | head -n 1)
    [ "$code" = "UNREACHABLE" ] && return 2
    echo "${code:-$out}"
    return 1
}

# 停掉后台循环并等它退出：TERM 只会打断两次心跳之间的等待；正在执行的 board beat 由 shell 等它执行完，
# 之后循环直接退出、不再发下一次。CLI 请求最长 30 秒，最多等 35 秒，超时强制结束
stop_loop() {
    f=$(pidfile "$1")
    [ -f "$f" ] || return 0
    pid=$(cat "$f")
    rm -f "$f"
    # 只停自己的后台循环（进程号可能已经被别的进程复用）
    case "$(ps -o command= -p "$pid" 2>/dev/null)" in
        *heartbeat.sh*_loop*) ;;
        *) return 0 ;;
    esac
    kill -TERM "$pid" 2>/dev/null || return 0
    waited=0
    while kill -0 "$pid" 2>/dev/null; do
        if [ "$waited" -ge 350 ]; then
            kill -KILL "$pid" 2>/dev/null
            break
        fi
        sleep 0.1 2>/dev/null || sleep 1
        waited=$((waited + 1))
    done
}

cmd=${1:-}
id=${2:-}
id=${id#\#}

case "$cmd" in
    start)
        [ -n "$id" ] || die "用法：heartbeat.sh start <ID>"
        command -v board >/dev/null 2>&1 || die "找不到 board 命令"
        stop_loop "$id"
        if ! err=$(beat_once "$id"); then
            [ -n "$err" ] || err="UNREACHABLE"
            die "第一次心跳失败：$err"
        fi
        watch=$(agent_pid)
        nohup sh "$0" _loop "$id" "${watch:-}" >/dev/null 2>&1 &
        echo $! >"$(pidfile "$id")"
        echo "✓ #$id 心跳已开始：每 ${INTERVAL} 秒一次（后台进程 $!${watch:+，跟随 agent 进程 $watch}）"
        ;;
    stop)
        [ -n "$id" ] || die "用法：heartbeat.sh stop <ID>"
        stop_loop "$id"
        board beat "$id" --final
        ;;
    _loop)
        watch=${3:-}
        me=$$
        nap=
        # 不管怎么退出，都删掉自己的 pid 文件（被 stop 停掉时 stop 已经删了）
        # 收到 TERM 时：在等待中就立即退出；正在发心跳时 shell 先等这次 board beat 结束再执行 trap，然后退出
        bye() {
            f=$(pidfile "$id")
            [ "$(cat "$f" 2>/dev/null)" = "$me" ] && rm -f "$f"
            exit 0
        }
        trap '[ -n "$nap" ] && kill "$nap" 2>/dev/null; bye' TERM INT
        while :; do
            sleep "$INTERVAL" &
            nap=$!
            wait "$nap"
            if [ -n "$watch" ] && ! kill -0 "$watch" 2>/dev/null; then
                bye # agent 不在了：不发 final，交给服务端判定异常退出
            fi
            beat_once "$id" >/dev/null
            [ $? -eq 1 ] && bye
        done
        ;;
    *)
        die "用法：heartbeat.sh start <ID> | stop <ID>"
        ;;
esac

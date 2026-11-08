# Board 心跳（Windows PowerShell 版，与 heartbeat.sh 行为相同）
#
# 一个 agent 会话只有一路后台心跳，每 10 秒执行一次 board beat（不带 ID），同时维持名字在线和这个名字持有的卡片。
# 名字从环境变量 BOARD_AS 读：
#
#   $env:BOARD_AS='<名字>'; powershell -NoProfile -ExecutionPolicy Bypass -File heartbeat.ps1 start
#   $env:BOARD_AS='<名字>'; powershell -NoProfile -ExecutionPolicy Bypass -File heartbeat.ps1 stop
#   $env:BOARD_AS='<名字>'; $env:BOARD='<项目>/<白板>'; powershell -NoProfile -ExecutionPolicy Bypass -File heartbeat.ps1 final <ID>
#
# start：check 之后执行一次（整个会话只执行这一次）：先同步发一次 board beat，失败直接报错退出；成功后进入后台循环
# stop ：会话结束：停掉后台循环，再发 board beat --final，名字立刻离线。
#        先放一个停止标记，等后台循环真正退出（正在执行的那次 board beat 会先发完），再发 final，
#        保证 final 是最后一条心跳。不能直接 Stop-Process：它只结束循环进程，已经启动的 board.exe 还会把心跳发出去
# final：卡片做完：board beat <ID> --final，结束这一次计时；之后 50 秒内要沿连线移走
# 后台循环在 agent 进程不在了、或 board beat 被拒绝（名字已离线等）时自己停下，不发 final；连不上服务端时继续重试。
# agent 进程：往上找第一个不是 shell 的祖先进程，可以用 BOARD_HEARTBEAT_PID 指定。pid 文件按名字区分。

param(
    [Parameter(Mandatory = $true, Position = 0)][string]$Cmd,
    # final：卡片 ID；_loop：要跟随的 agent 进程
    [Parameter(Position = 1)][string]$Arg = ''
)

$ErrorActionPreference = 'Stop'
$Interval = if ($env:BOARD_HEARTBEAT_INTERVAL) { [int]$env:BOARD_HEARTBEAT_INTERVAL } else { 10 } # 只给测试用；正常使用不要改

function NeedName {
    if (-not $env:BOARD_AS) { [Console]::Error.WriteLine("heartbeat: 没有名字：先 `$env:BOARD_AS='<名字>'"); exit 1 }
}

function PidFile {
    $dir = Join-Path ([IO.Path]::GetTempPath()) 'board-heartbeat'
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $key = "$env:BOARD_AS" -replace '[^A-Za-z0-9._-]', '_'
    Join-Path $dir "$key.pid"
}

function AgentPid {
    if ($env:BOARD_HEARTBEAT_PID) { return [int]$env:BOARD_HEARTBEAT_PID }
    $shells = @('powershell.exe', 'pwsh.exe', 'cmd.exe', 'bash.exe', 'sh.exe', 'conhost.exe', 'wsl.exe')
    $p = (Get-CimInstance Win32_Process -Filter "ProcessId=$PID").ParentProcessId
    while ($p -and $p -gt 4) {
        $proc = Get-CimInstance Win32_Process -Filter "ProcessId=$p"
        if (-not $proc) { return $null }
        if ($shells -notcontains $proc.Name.ToLower()) { return [int]$p }
        $p = $proc.ParentProcessId
    }
    return $null
}

# 发一次 agent 级心跳：成功返回 'OK'，连不上返回 'UNREACHABLE'，被拒绝返回错误码
function BeatOnce {
    $out = & board beat --json 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { return 'OK' }
    try { return ($out | ConvertFrom-Json).error.code } catch { return $out.Trim() }
}

# 停止标记：后台循环每 200ms 看一次，看到就退出；正在执行的 board beat 先让它执行完
function StopFile { "$(PidFile).stop" }

# 停掉后台循环并等它退出。CLI 请求最长 30 秒，最多等 35 秒，超时连同它启动的 board 一起强制结束
function StopLoop {
    $f = PidFile
    $stop = StopFile
    if (-not (Test-Path $f)) { Remove-Item $stop -Force -ErrorAction SilentlyContinue; return }
    $loop = [int](Get-Content $f)
    # 只停自己的后台循环（进程号可能已经被别的进程复用）
    $proc = Get-CimInstance Win32_Process -Filter "ProcessId=$loop"
    if ($proc -and $proc.CommandLine -like '*heartbeat.ps1*_loop*') {
        Set-Content -Path $stop -Value $loop
        try {
            Wait-Process -Id $loop -Timeout 35 -ErrorAction Stop
        } catch {
            Get-CimInstance Win32_Process -Filter "ParentProcessId=$loop" | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
            Stop-Process -Id $loop -Force -ErrorAction SilentlyContinue
        }
    }
    Remove-Item $f, $stop -Force -ErrorAction SilentlyContinue
}

switch ($Cmd) {
    'start' {
        NeedName
        if (-not (Get-Command board -ErrorAction SilentlyContinue)) { [Console]::Error.WriteLine('heartbeat: 找不到 board 命令'); exit 1 }
        StopLoop
        $r = BeatOnce
        if ($r -ne 'OK') { [Console]::Error.WriteLine("heartbeat: 第一次心跳失败：$r"); exit 1 }
        $agent = AgentPid
        $exe = (Get-Process -Id $PID).Path
        $argList = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '_loop')
        if ($agent) { $argList += "$agent" }
        $loop = Start-Process -FilePath $exe -ArgumentList $argList -WindowStyle Hidden -PassThru
        Set-Content -Path (PidFile) -Value $loop.Id
        $follow = if ($agent) { "，跟随 agent 进程 $agent" } else { '' }
        Write-Output "✓ $env:BOARD_AS 心跳已开始：每 $Interval 秒一次 board beat（后台进程 $($loop.Id)$follow）"
    }
    'stop' {
        NeedName
        StopLoop
        & board beat --final
        exit $LASTEXITCODE
    }
    'final' {
        NeedName
        $id = $Arg.TrimStart('#')
        if (-not $id) { [Console]::Error.WriteLine('用法：heartbeat.ps1 final <ID>'); exit 1 }
        & board beat $id --final
        exit $LASTEXITCODE
    }
    '_loop' {
        $watch = $Arg
        $f = PidFile
        $stop = StopFile
        try {
            :beat while ($true) {
                # 分小段睡，随时响应 stop
                $until = (Get-Date).AddSeconds($Interval)
                while ((Get-Date) -lt $until) {
                    if (Test-Path $stop) { break beat }
                    Start-Sleep -Milliseconds 200
                }
                if (Test-Path $stop) { break }
                # agent 不在了：不发 final，交给服务端判定名字离线、卡片心跳中断
                if ($watch -and -not (Get-Process -Id ([int]$watch) -ErrorAction SilentlyContinue)) { break }
                $r = BeatOnce
                if ($r -ne 'OK' -and $r -ne 'UNREACHABLE') { break }
            }
        } finally {
            if ((Test-Path $f) -and ((Get-Content $f) -eq "$PID")) { Remove-Item $f -Force -ErrorAction SilentlyContinue }
        }
    }
    default {
        [Console]::Error.WriteLine('用法：heartbeat.ps1 start | stop | final <ID>（名字从 BOARD_AS 读）')
        exit 1
    }
}

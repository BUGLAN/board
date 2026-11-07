# Board 心跳（Windows PowerShell 版，与 heartbeat.sh 行为相同）
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File heartbeat.ps1 start <ID>
#   powershell -NoProfile -ExecutionPolicy Bypass -File heartbeat.ps1 stop  <ID>
#
# start：先同步发一次心跳（失败直接报错退出），再在后台每 10 秒执行一次 board beat <ID>
# stop ：停掉后台循环，发送 final（正常退出）
# 后台循环在 agent 进程不在了、或 board beat 被拒绝时自己停下，不发 final；连不上服务端时继续重试。
# agent 进程：往上找第一个不是 shell 的祖先进程，可以用 BOARD_HEARTBEAT_PID 指定。

param(
    [Parameter(Mandatory = $true, Position = 0)][string]$Cmd,
    [Parameter(Mandatory = $true, Position = 1)][string]$Id,
    [Parameter(Position = 2)][string]$Watch = ''
)

$ErrorActionPreference = 'Stop'
$Interval = 10
$Id = $Id.TrimStart('#')

function PidFile {
    $dir = Join-Path ([IO.Path]::GetTempPath()) 'board-heartbeat'
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $key = ("{0}-{1}-{2}" -f $env:BOARD, $Id, $env:BOARD_AS) -replace '[^A-Za-z0-9._-]', '_'
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

# 发一次心跳：成功返回 'OK'，连不上返回 'UNREACHABLE'，被拒绝返回错误码
function BeatOnce {
    $out = & board beat $Id --json 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { return 'OK' }
    try { return ($out | ConvertFrom-Json).error.code } catch { return $out.Trim() }
}

function StopLoop {
    $f = PidFile
    if (-not (Test-Path $f)) { return }
    $loop = [int](Get-Content $f)
    # 只停自己的后台循环（进程号可能已经被别的进程复用）
    $proc = Get-CimInstance Win32_Process -Filter "ProcessId=$loop"
    if ($proc -and $proc.CommandLine -like '*heartbeat.ps1*_loop*') { Stop-Process -Id $loop -Force -ErrorAction SilentlyContinue }
    Remove-Item $f -Force -ErrorAction SilentlyContinue
}

switch ($Cmd) {
    'start' {
        if (-not (Get-Command board -ErrorAction SilentlyContinue)) { Write-Error 'heartbeat: 找不到 board 命令'; exit 1 }
        StopLoop
        $r = BeatOnce
        if ($r -ne 'OK') { [Console]::Error.WriteLine("heartbeat: 第一次心跳失败：$r"); exit 1 }
        $agent = AgentPid
        $exe = (Get-Process -Id $PID).Path
        $argList = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '_loop', $Id, "$agent")
        $loop = Start-Process -FilePath $exe -ArgumentList $argList -WindowStyle Hidden -PassThru
        Set-Content -Path (PidFile) -Value $loop.Id
        $follow = if ($agent) { "，跟随 agent 进程 $agent" } else { '' }
        Write-Output "✓ #$Id 心跳已开始：每 $Interval 秒一次（后台进程 $($loop.Id)$follow）"
    }
    'stop' {
        StopLoop
        & board beat $Id --final
        exit $LASTEXITCODE
    }
    '_loop' {
        $f = PidFile
        try {
            while ($true) {
                Start-Sleep -Seconds $Interval
                # agent 不在了：不发 final，交给服务端判定异常退出
                if ($Watch -and -not (Get-Process -Id ([int]$Watch) -ErrorAction SilentlyContinue)) { break }
                $r = BeatOnce
                if ($r -ne 'OK' -and $r -ne 'UNREACHABLE') { break }
            }
        } finally {
            if ((Test-Path $f) -and ((Get-Content $f) -eq "$PID")) { Remove-Item $f -Force -ErrorAction SilentlyContinue }
        }
    }
    default {
        [Console]::Error.WriteLine('用法：heartbeat.ps1 start <ID> | stop <ID>')
        exit 1
    }
}

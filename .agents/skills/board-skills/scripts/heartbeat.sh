#!/bin/sh
# 兼容旧写法：心跳已经整合进 board 命令行，这个脚本只是转发给 skill 自带的 board。
#   BOARD_AS=<名字> sh heartbeat.sh start | stop             → board heartbeat start | stop
#   BOARD_AS=<名字> BOARD=<项目>/<白板> sh heartbeat.sh final <ID>   → board heartbeat final <ID>
# 新写法见 SKILL.md：<skill 目录>/scripts/board heartbeat start --as <名字>
exec sh "$(dirname "$0")/board" heartbeat "$@"

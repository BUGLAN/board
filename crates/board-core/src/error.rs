//! 错误码（PRD §12）。CLI 被拒绝时输出错误码和原因，退出码为 1。

use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    NoIdentity,
    NoBoard,
    BoardNotFound,
    NotFound,
    EmptyTitle,
    EmptyTodo,
    NotInTodo,
    AlreadyHolding,
    NotHolder,
    ForbiddenColumn,
    Usage,
    /// 连接服务端时没有登录：没有会话也没有令牌，或令牌不能用于这个操作
    NotLoggedIn,
    /// 访问令牌不存在或已吊销
    InvalidToken,
    /// CLI 连不上服务端
    Unreachable,
    /// agent 名字正被在线 agent 使用（PRD §3）
    NameTaken,
    /// agent 名字不在线：没有 check，或心跳中断超过 50 秒（PRD §3）
    NameOffline,
    /// 存储层故障（磁盘、数据库损坏等），不属于规则拒绝
    Internal,
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 16] = [
        ErrorCode::NoIdentity,
        ErrorCode::NoBoard,
        ErrorCode::BoardNotFound,
        ErrorCode::NotFound,
        ErrorCode::EmptyTitle,
        ErrorCode::EmptyTodo,
        ErrorCode::NotInTodo,
        ErrorCode::AlreadyHolding,
        ErrorCode::NotHolder,
        ErrorCode::ForbiddenColumn,
        ErrorCode::Usage,
        ErrorCode::NotLoggedIn,
        ErrorCode::InvalidToken,
        ErrorCode::Unreachable,
        ErrorCode::NameTaken,
        ErrorCode::NameOffline,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::NoIdentity => "NO_IDENTITY",
            ErrorCode::NoBoard => "NO_BOARD",
            ErrorCode::BoardNotFound => "BOARD_NOT_FOUND",
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::EmptyTitle => "EMPTY_TITLE",
            ErrorCode::EmptyTodo => "EMPTY_TODO",
            ErrorCode::NotInTodo => "NOT_IN_TODO",
            ErrorCode::AlreadyHolding => "ALREADY_HOLDING",
            ErrorCode::NotHolder => "NOT_HOLDER",
            ErrorCode::ForbiddenColumn => "FORBIDDEN_COLUMN",
            ErrorCode::Usage => "USAGE",
            ErrorCode::NotLoggedIn => "NOT_LOGGED_IN",
            ErrorCode::InvalidToken => "INVALID_TOKEN",
            ErrorCode::Unreachable => "UNREACHABLE",
            ErrorCode::NameTaken => "NAME_TAKEN",
            ErrorCode::NameOffline => "NAME_OFFLINE",
            ErrorCode::Internal => "INTERNAL",
        }
    }

    /// 从错误码文本还原（CLI 远程模式读取服务端返回的错误）
    pub fn parse(s: &str) -> Option<ErrorCode> {
        ErrorCode::ALL.into_iter().chain([ErrorCode::Internal]).find(|c| c.as_str() == s)
    }

    /// 默认原因
    pub fn message(self) -> &'static str {
        match self {
            ErrorCode::NoIdentity => "没有提供 agent 名字：先 board check --as <名字>，之后每条命令都带 --as <名字>",
            ErrorCode::NoBoard => "没有指定白板：使用 --board <项目>/<白板> 或设置环境变量 BOARD",
            ErrorCode::BoardNotFound => "白板不存在，用 board boards 查看可用的白板",
            ErrorCode::NotFound => "卡片不存在",
            ErrorCode::EmptyTitle => "标题不能为空",
            ErrorCode::EmptyTodo => "可领取的状态里没有能领取的卡片",
            ErrorCode::NotInTodo => "这张卡片不在可领取的状态，也不是你持有的卡片",
            ErrorCode::AlreadyHolding => "你已经持有另一张卡片，一个 agent 在一块白板上同时只能持有一张",
            ErrorCode::NotHolder => "被持有的卡片只有持有者能编辑和移动",
            ErrorCode::ForbiddenColumn => "不允许这样移动：锁定状态只能领取进入，终点和「未整理」只归人管，agent 只能沿连线移动",
            ErrorCode::Usage => "命令格式或参数不正确",
            ErrorCode::NotLoggedIn => "未登录：网页端请先用 GitHub 登录；命令行运行 board login",
            ErrorCode::InvalidToken => "访问令牌无效或已吊销：在界面「设置 → Agent 接入」里新建令牌",
            ErrorCode::Unreachable => "连不上 Board 服务端",
            ErrorCode::NameTaken => "名字正被在线 agent 使用：加一个和本会话相关的后缀（例如任务名或分支名）再 check",
            ErrorCode::NameOffline => "名字不在线：没有 check，或心跳中断超过 50 秒，先 board check --as <名字>",
            ErrorCode::Internal => "内部错误",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BoardError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl BoardError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        BoardError { code, message: message.into(), detail: None }
    }
    pub fn code(code: ErrorCode) -> Self {
        BoardError::new(code, code.message())
    }
    pub fn usage(message: impl Into<String>) -> Self {
        BoardError::new(ErrorCode::Usage, message)
    }
    pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }
}

impl fmt::Display for BoardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BoardError {}

impl From<rusqlite::Error> for BoardError {
    fn from(e: rusqlite::Error) -> Self {
        BoardError::new(ErrorCode::Internal, format!("数据库错误：{e}"))
    }
}

impl From<std::io::Error> for BoardError {
    fn from(e: std::io::Error) -> Self {
        BoardError::new(ErrorCode::Internal, format!("文件错误：{e}"))
    }
}

pub type Result<T> = std::result::Result<T, BoardError>;

//! 任务的展示模型与倒计时计算（与网络、UI 无关的纯逻辑）。

use chrono::{DateTime, Local};

/// Mower 后端的运行状态（来自 `/status` 接口）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MowerStatus {
    /// 后端在线但 mower 未运行。
    Stopped,
    /// mower 正在任务间休眠。
    Sleeping,
    /// mower 正在执行任务。
    Working,
    /// 还未取得状态（启动初期）。
    Unknown,
}

impl MowerStatus {
    /// 用于角标展示的中文标签；`Unknown` 返回空串表示不展示。
    pub fn label(self) -> &'static str {
        match self {
            MowerStatus::Stopped => "已停止",
            MowerStatus::Sleeping => "休眠中",
            MowerStatus::Working => "运行中",
            MowerStatus::Unknown => "",
        }
    }
}

/// 当前展示的任务信息（含可选的下一个任务）。
#[derive(Debug, Clone)]
pub struct TaskInfo {
    pub current_name: String,
    pub current_time: DateTime<Local>,
    pub current_countdown: String,
    pub is_expired: bool,
    /// 剩余秒数，用于判断是否需要提醒。
    pub remaining_seconds: i64,
    /// 跑单任务的目标房间（已翻译，如 `B101`）；非跑单或无 plan 时为 `None`。
    pub current_rooms: Option<String>,
    pub next_task: Option<NextTaskInfo>,
}

/// 下一个任务的信息。
#[derive(Debug, Clone)]
pub struct NextTaskInfo {
    pub name: String,
    pub target_time: DateTime<Local>,
    pub countdown: String,
}

/// 计算倒计时字符串和状态，返回 `(倒计时文本, 是否已过期, 剩余秒数)`。
pub fn calculate_countdown_with_status(target_time: &DateTime<Local>) -> (String, bool, i64) {
    let now = Local::now();
    let duration = target_time.signed_duration_since(now);
    let remaining_secs = duration.num_seconds();

    if remaining_secs <= 0 {
        // 已过期，只显示“执行中”，不显示已执行时间
        ("执行中".to_string(), true, remaining_secs)
    } else {
        let hours = duration.num_hours();
        let minutes = duration.num_minutes() % 60;
        let seconds = duration.num_seconds() % 60;
        (
            format!("{:02}:{:02}:{:02}", hours, minutes, seconds),
            false,
            remaining_secs,
        )
    }
}

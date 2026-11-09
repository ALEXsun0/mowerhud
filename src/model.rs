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
    /// mower 正在启动或等待初始化完成。
    Starting,
    /// 设备正在恢复或暂停后等待恢复。
    Recovering,
    /// 还未取得状态（启动初期）。
    Unknown,
}

impl MowerStatus {
    pub fn from_api(value: &str) -> Self {
        match value {
            "stopped" => Self::Stopped,
            "sleeping" => Self::Sleeping,
            "working" => Self::Working,
            "starting" => Self::Starting,
            "recovering" => Self::Recovering,
            _ => Self::Unknown,
        }
    }

    /// 用于角标展示的中文标签；`Unknown` 返回空串表示不展示。
    pub fn label(self) -> &'static str {
        match self {
            MowerStatus::Stopped => "已停止",
            MowerStatus::Sleeping => "休眠中",
            MowerStatus::Working => "运行中",
            MowerStatus::Starting => "启动中",
            MowerStatus::Recovering => "恢复中",
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
        // 队列接口未提供当前执行项，时间已到不代表该项一定正在执行。
        ("已到期".to_string(), true, remaining_secs)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overdue_queued_task_does_not_claim_to_be_executing() {
        let time = Local::now() - chrono::Duration::minutes(2);
        let (label, expired, _) = calculate_countdown_with_status(&time);
        assert!(expired);
        assert_eq!(label, "已到期");
    }
}

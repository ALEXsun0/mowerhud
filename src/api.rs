//! 与割草机 API 的网络交互：拉取状态/任务、测试连接、后台轮询。

use crate::config::Config;
use crate::model::{MowerStatus, NextTaskInfo, TaskInfo, calculate_countdown_with_status};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::interval;

/// 任何线程安全的错误。`Send + Sync` 让它能跨 `async`/`spawn` 边界传递。
type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// API 返回的原始任务结构 —— 只保留我们关心的字段。
#[derive(Deserialize, Debug, Clone)]
struct Task {
    /// ISO 8601 格式时间，例如 `2025-10-09T20:19:40.257970`
    time: String,
    #[serde(rename = "type")]
    task_type: TaskType,
    /// 泛化任务（“空任务”）的真实名称藏在这里。
    #[serde(default)]
    meta_data: String,
    /// 房间 -> 干员列表；跑单任务用它定位贸易站。值的具体形状我们不关心。
    #[serde(default)]
    plan: Option<BTreeMap<String, serde_json::Value>>,
}

#[derive(Deserialize, Debug, Clone)]
struct TaskType {
    display_value: String,
    /// 优先级：跑单为 1，其余为 2。
    #[serde(default)]
    priority: i64,
}

/// `/status` 返回体 —— 只取 `status` 字段。
#[derive(Deserialize, Debug)]
struct StatusResponse {
    #[serde(default)]
    status: String,
}

/// 构造带超时的客户端，避免挂死连接卡住整个轮询。
fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// 构造 GET 请求，带上 Mower token（HTTP 头 `token`）与可选的 Basic 认证。
///
/// token 是 Mower 自带的鉴权（`config.conf.webview.token`）；Basic 认证是给反向代理用的。
/// 两者相互独立，可同时存在。`/status`、`/task` 本身不校验 token，多带也无害。
fn authed_get(client: &reqwest::Client, config: &Config, path: &str) -> reqwest::RequestBuilder {
    let url = format!("{}{}", config.base_url(), path);
    let mut request = client.get(&url);
    if let Some(token) = &config.token
        && !token.is_empty()
    {
        request = request.header("token", token);
    }
    if let (Some(username), Some(password)) = (&config.username, &config.password) {
        request = request.basic_auth(username, Some(password));
    }
    request
}

/// 每秒拉取一次状态/任务数据，更新共享状态。复用单个 [`reqwest::Client`] 以享受连接池。
///
/// 状态机：
/// - `/status` 不可达 → 离线，写入 `task_error`，但**保留**上次任务与状态（不闪错误屏）。
/// - `status == stopped` → mower 已停止，清空任务与错误。
/// - 其余 → 拉取 `/task`；成功则更新任务并清错，失败则记错但保留上次任务。
pub async fn fetch_state_loop(
    task_info: Arc<Mutex<Option<TaskInfo>>>,
    task_error: Arc<Mutex<Option<String>>>,
    mower_status: Arc<Mutex<MowerStatus>>,
    config: Config,
) {
    let client = build_client();
    let mut interval = interval(Duration::from_secs(1));

    loop {
        interval.tick().await;

        match fetch_status(&client, &config).await {
            Ok(status) => {
                if let Ok(mut guard) = mower_status.lock() {
                    *guard = status;
                }
                if status == MowerStatus::Stopped {
                    // 已停止：没有任务，清空残留任务与错误，让 UI 展示“已停止”。
                    if let Ok(mut guard) = task_info.lock() {
                        *guard = None;
                    }
                    if let Ok(mut guard) = task_error.lock() {
                        *guard = None;
                    }
                } else {
                    match fetch_earliest_task(&client, &config).await {
                        Ok(task) => {
                            if let Ok(mut guard) = task_info.lock() {
                                *guard = Some(task);
                            }
                            if let Ok(mut guard) = task_error.lock() {
                                *guard = None;
                            }
                        }
                        Err(e) => {
                            if let Ok(mut guard) = task_error.lock() {
                                *guard = Some(e.to_string());
                            }
                        }
                    }
                }
            }
            Err(e) => {
                // 整个后端不可达：保留上次任务与状态，仅标记离线。
                if let Ok(mut guard) = task_error.lock() {
                    *guard = Some(e.to_string());
                }
            }
        }
    }
}

/// 拉取 mower 运行状态。
async fn fetch_status(client: &reqwest::Client, config: &Config) -> Result<MowerStatus, BoxError> {
    let response = authed_get(client, config, "/status").send().await?;
    let status: StatusResponse = response.json().await?;
    Ok(match status.status.as_str() {
        "stopped" => MowerStatus::Stopped,
        "sleeping" => MowerStatus::Sleeping,
        "working" => MowerStatus::Working,
        _ => MowerStatus::Unknown,
    })
}

/// 解析后端时间字符串为本地时间，尽量宽容以免静默丢任务。
fn parse_task_time(s: &str) -> Option<DateTime<Local>> {
    // 优先按 RFC3339（带时区/Z）解析。
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Local));
    }
    // 否则按朴素本地时间，允许有或没有小数秒。
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            // DST 歧义时取较早的一个，而非像 .single() 那样直接丢弃。
            return Local.from_local_datetime(&naive).earliest();
        }
    }
    None
}

/// 任务展示名：镜像后端 `SchedulerTask.format()`，“空任务”用 `meta_data` 兜底。
fn task_label(task: &Task) -> String {
    if task.task_type.display_value == "空任务" && !task.meta_data.is_empty() {
        task.meta_data.clone()
    } else {
        task.task_type.display_value.clone()
    }
}

/// 把后端房间标识翻译成中文，镜像后端 `translate_room`。
fn translate_room(room: &str) -> String {
    let parts: Vec<&str> = room.split('_').collect();
    if room.contains("dormitory") {
        return format!("{}层宿舍", parts.get(1).copied().unwrap_or(""));
    }
    if room.contains("central") {
        return "控制中枢".to_string();
    }
    if room.contains("factory") {
        return "加工站".to_string();
    }
    if room.contains("meeting") {
        return "会客室".to_string();
    }
    if room.contains("contact") {
        return "办公室".to_string();
    }
    if room.contains("train") {
        return "训练室".to_string();
    }
    if room.contains("room")
        && let (Some(a), Some(b)) = (parts.get(1), parts.get(2))
    {
        return format!("B{}0{}", a, b);
    }
    room.to_string()
}

/// 跑单任务（priority==1）才返回目标房间字符串，否则 `None`。
fn run_order_rooms(task: &Task) -> Option<String> {
    if task.task_type.priority != 1 {
        return None;
    }
    let plan = task.plan.as_ref()?;
    if plan.is_empty() {
        return None;
    }
    let rooms: Vec<String> = plan.keys().map(|r| translate_room(r)).collect();
    Some(rooms.join("、"))
}

/// 拉取并计算「当前 + 下一个」任务。
async fn fetch_earliest_task(
    client: &reqwest::Client,
    config: &Config,
) -> Result<TaskInfo, BoxError> {
    let response = authed_get(client, config, "/task").send().await?;
    let tasks: Vec<Task> = response.json().await?;

    let now = Local::now();

    // 解析时间并保留有效任务
    let mut parsed_tasks: Vec<(Task, DateTime<Local>)> = tasks
        .into_iter()
        .filter_map(|task| parse_task_time(&task.time).map(|t| (task, t)))
        .collect();

    if parsed_tasks.is_empty() {
        return Err("没有找到有效任务".into());
    }

    parsed_tasks.sort_by_key(|(_, time)| *time);

    // 选择当前应展示的任务：
    // - 有未过期任务且它前面有任务 → 展示前一个（执行中）
    // - 有未过期任务且它是第一个 → 从第一个开始
    // - 全部过期 → 展示最后一个（执行中）
    let first_future_idx = parsed_tasks.iter().position(|(_, time)| time >= &now);
    let current_index = match first_future_idx {
        Some(future_idx) if future_idx > 0 => future_idx - 1,
        Some(_) => 0,
        None => parsed_tasks.len().saturating_sub(1),
    };

    let (current_task, current_time) = &parsed_tasks[current_index];
    let (current_countdown, is_expired, remaining_secs) =
        calculate_countdown_with_status(current_time);
    let current_rooms = run_order_rooms(current_task);

    // 下一个任务：只要后面还有就展示
    let next_task = parsed_tasks.get(current_index + 1).map(|(task, time)| {
        let (countdown, _, _) = calculate_countdown_with_status(time);
        NextTaskInfo {
            name: task_label(task),
            target_time: *time,
            countdown,
        }
    });

    Ok(TaskInfo {
        current_name: task_label(current_task),
        current_time: *current_time,
        current_countdown,
        is_expired,
        remaining_seconds: remaining_secs,
        current_rooms,
        next_task,
    })
}

/// 测试 API 连接是否可用。
pub async fn test_api_connection(config: &Config) -> Result<(), BoxError> {
    let client = build_client();
    let response = authed_get(&client, config, "/status").send().await?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP错误: {}", response.status()).into())
    }
}

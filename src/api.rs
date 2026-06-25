//! 与割草机 API 的网络交互：拉取任务、测试连接、后台轮询。

use crate::config::Config;
use crate::model::{NextTaskInfo, TaskInfo, calculate_countdown_with_status};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::interval;

/// API 返回的原始任务结构 —— 只保留我们关心的字段。
#[derive(Deserialize, Debug, Clone)]
struct Task {
    /// ISO 8601 格式时间，例如 `2025-10-09T20:19:40.257970`
    time: String,
    #[serde(rename = "type")]
    task_type: TaskType,
}

#[derive(Deserialize, Debug, Clone)]
struct TaskType {
    display_value: String,
}

/// 每秒拉取一次任务数据，更新共享状态。复用单个 [`reqwest::Client`] 以享受连接池。
pub async fn fetch_tasks_loop(
    task_info: Arc<Mutex<Option<TaskInfo>>>,
    task_error: Arc<Mutex<Option<String>>>,
    config: Config,
) {
    let client = reqwest::Client::new();
    let mut interval = interval(Duration::from_secs(1));

    loop {
        interval.tick().await;

        match fetch_earliest_task(&client, &config).await {
            Ok(task) => {
                if let Ok(mut guard) = task_info.lock() {
                    *guard = Some(task);
                }
                if let Ok(mut error_guard) = task_error.lock() {
                    *error_guard = None;
                }
            }
            Err(e) => {
                if let Ok(mut error_guard) = task_error.lock() {
                    *error_guard = Some(e.to_string());
                }
            }
        }
    }
}

/// 拉取并计算「当前 + 下一个」任务。
async fn fetch_earliest_task(
    client: &reqwest::Client,
    config: &Config,
) -> Result<TaskInfo, Box<dyn std::error::Error>> {
    let url = format!("{}/task", config.base_url());

    let mut request = client.get(&url);
    if let (Some(username), Some(password)) = (&config.username, &config.password) {
        request = request.basic_auth(username, Some(password));
    }

    let response = request.send().await?;
    let tasks: Vec<Task> = response.json().await?;

    let now = Local::now();

    // 解析时间并保留有效任务
    let mut parsed_tasks: Vec<(Task, DateTime<Local>)> = tasks
        .into_iter()
        .filter_map(|task| {
            NaiveDateTime::parse_from_str(&task.time, "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .and_then(|naive_dt| Local.from_local_datetime(&naive_dt).single())
                .map(|local_dt| (task, local_dt))
        })
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

    // 下一个任务：只要后面还有就展示
    let next_task = parsed_tasks.get(current_index + 1).map(|(task, time)| {
        let (countdown, _, _) = calculate_countdown_with_status(time);
        NextTaskInfo {
            name: task.task_type.display_value.clone(),
            target_time: *time,
            countdown,
        }
    });

    Ok(TaskInfo {
        current_name: current_task.task_type.display_value.clone(),
        current_time: *current_time,
        current_countdown,
        is_expired,
        remaining_seconds: remaining_secs,
        next_task,
    })
}

/// 测试 API 连接是否可用。
pub async fn test_api_connection(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let url = format!("{}/task", config.base_url());

    let mut request = client.get(&url);
    if let (Some(username), Some(password)) = (&config.username, &config.password) {
        request = request.basic_auth(username, Some(password));
    }

    let response = request.send().await?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP错误: {}", response.status()).into())
    }
}

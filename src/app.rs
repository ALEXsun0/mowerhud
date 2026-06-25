//! 应用状态机：持有全局状态、驱动每帧逻辑、桥接 UI 与后台任务。

use crate::api;
use crate::config::{Config, load_config, save_config};
use crate::model::{TaskInfo, calculate_countdown_with_status};
use crate::notify::{play_beep_sound, send_notification};
use crate::ui;
use eframe::egui;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 提前 2 分钟提醒的阈值（秒）。
const NOTIFY_AHEAD_2MIN: i64 = 120;
/// 提前 30 秒提醒的阈值（秒）。
const NOTIFY_AHEAD_30SEC: i64 = 30;

/// 应用所处界面。
pub enum AppState {
    Configuration,
    Running,
}

/// 按开关发送提醒：通知与提示音相互独立。
fn fire_alert(notify_enabled: bool, sound_enabled: bool, task_name: &str, message: &str) {
    if notify_enabled {
        send_notification(task_name, message);
    }
    if sound_enabled {
        play_beep_sound();
    }
}

pub struct MowerHUD {
    pub state: AppState,
    pub config: Config,
    pub task_info: Arc<Mutex<Option<TaskInfo>>>,
    pub task_error: Arc<Mutex<Option<String>>>,
    pub last_update: Instant,
    // 配置界面临时字段
    pub temp_address: String,
    pub temp_use_https: bool,
    pub temp_username: String,
    pub temp_password: String,
    pub temp_notify_enabled: bool,
    pub temp_sound_enabled: bool,
    pub connection_status: Arc<Mutex<String>>,
    pub always_on_top: bool,
    // 通知状态
    pub notified_2min: bool,
    pub notified_30sec: bool,
    pub last_task_name: String,
    /// 上次发给系统的窗口尺寸，用于避免每帧重复发送 InnerSize。
    last_sent_size: Option<egui::Vec2>,
}

impl MowerHUD {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui::fonts::setup_chinese_fonts(&cc.egui_ctx);

        let config = load_config().unwrap_or_default();

        Self {
            state: AppState::Configuration,
            temp_address: format!("{}:{}", config.host, config.port),
            temp_use_https: config.use_https,
            temp_username: config.username.clone().unwrap_or_default(),
            temp_password: config.password.clone().unwrap_or_default(),
            temp_notify_enabled: config.notify_enabled,
            temp_sound_enabled: config.sound_enabled,
            config,
            task_info: Arc::new(Mutex::new(None)),
            task_error: Arc::new(Mutex::new(None)),
            last_update: Instant::now(),
            connection_status: Arc::new(Mutex::new(String::new())),
            always_on_top: true,
            notified_2min: false,
            notified_30sec: false,
            last_task_name: String::new(),
            last_sent_size: None,
        }
    }

    /// 启动后台任务轮询。
    pub fn start_task_fetching(&self) {
        let task_info = self.task_info.clone();
        let task_error = self.task_error.clone();
        let config = self.config.clone();
        tokio::spawn(async move {
            api::fetch_tasks_loop(task_info, task_error, config).await;
        });
    }

    /// 用临时字段测试 API 连接，结果写入 `connection_status`。
    pub fn test_connection(&mut self) {
        match self.build_config_from_temp() {
            Ok(config) => {
                if let Ok(mut status) = self.connection_status.lock() {
                    *status = "正在测试连接...".to_string();
                }
                let status_clone = self.connection_status.clone();
                tokio::spawn(async move {
                    let msg = match api::test_api_connection(&config).await {
                        Ok(_) => "连接测试成功！".to_string(),
                        Err(e) => format!("连接测试失败: {}", e),
                    };
                    if let Ok(mut status) = status_clone.lock() {
                        *status = msg;
                    }
                });
            }
            Err(e) => {
                if let Ok(mut status) = self.connection_status.lock() {
                    *status = e;
                }
            }
        }
    }

    /// 校验并保存配置；成功返回 true。
    pub fn save_and_start(&mut self) -> bool {
        match self.build_config_from_temp() {
            Ok(config) => {
                self.config = config;
                if let Err(e) = save_config(&self.config) {
                    self.set_status(format!("保存配置失败: {}", e));
                    return false;
                }
                self.set_status("配置已保存".to_string());
                true
            }
            Err(e) => {
                self.set_status(e);
                false
            }
        }
    }

    fn build_config_from_temp(&self) -> Result<Config, String> {
        let mut config = Config::from_address(
            &self.temp_address,
            self.temp_use_https,
            &self.temp_username,
            &self.temp_password,
        )?;
        config.notify_enabled = self.temp_notify_enabled;
        config.sound_enabled = self.temp_sound_enabled;
        Ok(config)
    }

    fn set_status(&self, msg: String) {
        if let Ok(mut status) = self.connection_status.lock() {
            *status = msg;
        }
    }

    /// 仅在窗口尺寸发生变化时才请求调整，避免每帧抖动。
    fn request_window_size(&mut self, ctx: &egui::Context, size: egui::Vec2) {
        if self.last_sent_size != Some(size) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            self.last_sent_size = Some(size);
        }
    }

    /// 每秒推进倒计时并按阈值发送提醒。
    fn tick(&mut self) {
        if self.last_update.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_update = Instant::now();

        // 提前取出提醒开关，避免在持有 task_info 锁时再借用 self
        let notify_enabled = self.config.notify_enabled;
        let sound_enabled = self.config.sound_enabled;

        let Ok(mut guard) = self.task_info.try_lock() else {
            return;
        };
        let Some(task) = guard.as_mut() else {
            return;
        };

        let (countdown, is_expired, remaining) =
            calculate_countdown_with_status(&task.current_time);
        task.current_countdown = countdown;
        task.is_expired = is_expired;
        task.remaining_seconds = remaining;

        // 任务变化则重置通知状态
        if self.last_task_name != task.current_name {
            self.last_task_name = task.current_name.clone();
            self.notified_2min = false;
            self.notified_30sec = false;
        }

        if !is_expired && remaining > 0 {
            if !self.notified_2min
                && remaining <= NOTIFY_AHEAD_2MIN
                && remaining > NOTIFY_AHEAD_30SEC
            {
                fire_alert(
                    notify_enabled,
                    sound_enabled,
                    &task.current_name,
                    "还有2分钟开始",
                );
                self.notified_2min = true;
            }
            if !self.notified_30sec && remaining <= NOTIFY_AHEAD_30SEC {
                fire_alert(
                    notify_enabled,
                    sound_enabled,
                    &task.current_name,
                    "还有30秒开始",
                );
                self.notified_30sec = true;
            }
        }

        // 倒计时回到 2 分钟外则重置通知状态，便于下一轮提醒
        if remaining > NOTIFY_AHEAD_2MIN {
            self.notified_2min = false;
            self.notified_30sec = false;
        }

        if let Some(next) = task.next_task.as_mut() {
            let (next_countdown, _, _) = calculate_countdown_with_status(&next.target_time);
            next.countdown = next_countdown;
        }
    }

    /// 当前任务是否还有下一个任务（用于决定 HUD 宽度）。
    fn has_next_task(&self) -> bool {
        self.task_info
            .try_lock()
            .ok()
            .and_then(|g| g.as_ref().map(|t| t.next_task.is_some()))
            .unwrap_or(false)
    }

    /// 倒计时是否进入需要流畅彩虹动画的窗口（2 分钟内）。
    fn needs_fast_repaint(&self) -> bool {
        self.task_info
            .try_lock()
            .ok()
            .and_then(|g| {
                g.as_ref().map(|t| {
                    !t.is_expired
                        && t.remaining_seconds > 0
                        && t.remaining_seconds <= ui::theme::RAINBOW_THRESHOLD_SECS
                })
            })
            .unwrap_or(false)
    }
}

impl eframe::App for MowerHUD {
    // eframe 0.34 用 `ui` 方法替代了已废弃的 `update`。
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        match self.state {
            AppState::Configuration => {
                crate::ui::config_view::show(self, ui);
                self.request_window_size(&ctx, egui::Vec2::from(ui::theme::CONFIG_SIZE));
            }
            AppState::Running => {
                crate::ui::task_view::show(self, ui);

                let width = if self.has_next_task() {
                    ui::theme::HUD_WIDTH_DUAL
                } else {
                    ui::theme::HUD_WIDTH_SINGLE
                };
                self.request_window_size(&ctx, egui::vec2(width, ui::theme::HUD_HEIGHT));

                self.tick();

                if self.needs_fast_repaint() {
                    // 2 分钟内持续重绘以实现流畅彩虹动画
                    ctx.request_repaint();
                } else {
                    ctx.request_repaint_after(Duration::from_secs(1));
                }
            }
        }
    }
}

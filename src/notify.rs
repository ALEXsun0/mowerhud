//! 桌面通知与提示音。
//!
//! 两者都在独立线程上执行：`notify_rust::show()` 与 `Beep` 都是阻塞调用，
//! 若放在 egui 渲染线程上会卡住整个 HUD（5 声 Beep 约冻结 0.9 秒）。

/// 发送桌面通知（任务提醒）。在后台线程执行，不阻塞 UI。
pub fn send_notification(task_name: &str, message: &str) {
    let task_name = task_name.to_string();
    let message = message.to_string();
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new()
            .summary("MowerHUD 任务提醒")
            .body(&format!("{}: {}", task_name, message))
            .show();
    });
}

/// 播放提示音。在后台线程执行，不阻塞 UI；目前仅 Windows 通过 `Beep` 实现。
pub fn play_beep_sound() {
    #[cfg(windows)]
    std::thread::spawn(|| unsafe {
        use winapi::um::utilapiset::Beep;
        // 连续 5 声：Beep(频率Hz, 持续ms)，间隔 100ms
        for i in 0..5 {
            Beep(800, 100);
            if i < 4 {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    });
}

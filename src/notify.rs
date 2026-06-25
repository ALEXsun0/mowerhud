//! 桌面通知与提示音。

/// 发送桌面通知（任务提醒）。
pub fn send_notification(task_name: &str, message: &str) {
    let _ = notify_rust::Notification::new()
        .summary("MowerHUD 任务提醒")
        .body(&format!("{}: {}", task_name, message))
        .show();
}

/// 播放提示音。目前仅在 Windows 上通过 `Beep` 实现，其他平台为空操作。
pub fn play_beep_sound() {
    #[cfg(windows)]
    unsafe {
        use winapi::um::utilapiset::Beep;
        // 连续 5 声：Beep(频率Hz, 持续ms)，间隔 100ms
        for i in 0..5 {
            Beep(800, 100);
            if i < 4 {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

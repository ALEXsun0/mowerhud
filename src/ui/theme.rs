//! 主题色与窗口布局常量。

use egui::Color32;

// ── 窗口尺寸 ──────────────────────────────────────────────
/// 配置界面宽度（高度按内容自适应）。
pub const CONFIG_WIDTH: f32 = 360.0;
/// 配置界面最小高度。
pub const CONFIG_MIN_HEIGHT: f32 = 200.0;
// HUD（运行界面）尺寸不再用固定常量：由 task_view 测量内容后自适应。

/// 进入彩虹变色提醒的剩余秒数阈值。
pub const RAINBOW_THRESHOLD_SECS: i64 = 120;

/// 根据剩余秒数计算倒计时颜色。
///
/// - 已过期：绿色
/// - 2 分钟内：RGB 彩虹循环（由 `time_secs` 驱动，单调递增的 egui 输入时间）
/// - 其余：红色
pub fn get_countdown_color(remaining_seconds: i64, is_expired: bool, time_secs: f64) -> Color32 {
    if is_expired {
        return Color32::from_rgb(100, 200, 100);
    }
    if remaining_seconds <= 0 || remaining_seconds > RAINBOW_THRESHOLD_SECS {
        return Color32::from_rgb(255, 100, 100);
    }

    // 彩虹循环，周期 2 秒
    let cycle = (time_secs.rem_euclid(2.0)) as f32 / 2.0; // 0.0..1.0
    let (r, g, b) = if cycle < 1.0 / 6.0 {
        // 红 -> 黄
        (1.0, cycle * 6.0, 0.0)
    } else if cycle < 2.0 / 6.0 {
        // 黄 -> 绿
        (1.0 - (cycle - 1.0 / 6.0) * 6.0, 1.0, 0.0)
    } else if cycle < 3.0 / 6.0 {
        // 绿 -> 青
        (0.0, 1.0, (cycle - 2.0 / 6.0) * 6.0)
    } else if cycle < 4.0 / 6.0 {
        // 青 -> 蓝
        (0.0, 1.0 - (cycle - 3.0 / 6.0) * 6.0, 1.0)
    } else if cycle < 5.0 / 6.0 {
        // 蓝 -> 紫
        ((cycle - 4.0 / 6.0) * 6.0, 0.0, 1.0)
    } else {
        // 紫 -> 红
        (1.0, 0.0, 1.0 - (cycle - 5.0 / 6.0) * 6.0)
    };

    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

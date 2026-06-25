//! 运行界面（HUD）：展示当前 / 下一个任务及倒计时。

use crate::app::{AppState, MowerHUD};
use crate::ui::theme;

pub fn show(app: &mut MowerHUD, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();

    // 拖动支持：在任意空白区域按住左键拖动可移动窗口
    if ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary))
        && ctx.input(|i| i.pointer.delta().length() > 0.0)
    {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }

    // 用 egui 输入时间驱动彩虹动画（单调递增，且与重绘同步）
    let time_secs = ctx.input(|i| i.time);

    egui::CentralPanel::default().show_inside(ui, |ui| {
        // 错误状态优先展示
        // 克隆出错误文本后立即释放锁，避免渲染期间持锁
        let error_msg = app.task_error.try_lock().ok().and_then(|g| g.clone());
        if let Some(error_msg) = error_msg {
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(&error_msg)
                        .size(12.0)
                        .color(egui::Color32::from_rgb(255, 150, 150)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(10.0);
                    if ui
                        .add_sized([70.0, 24.0], egui::Button::new("配置"))
                        .clicked()
                    {
                        app.state = AppState::Configuration;
                    }
                });
            });
            return;
        }

        // 任务展示
        if let Ok(task_guard) = app.task_info.try_lock() {
            if let Some(ref task) = *task_guard {
                ui.horizontal_centered(|ui| {
                    ui.add_space(5.0);

                    // 窗口控制按钮组（垂直排列）
                    ui.allocate_ui_with_layout(
                        egui::vec2(25.0, 50.0),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            if ui.add_sized([25.0, 20.0], egui::Button::new("X")).clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            ui.add_space(3.0);
                            if ui.add_sized([25.0, 20.0], egui::Button::new("—")).clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                            }
                        },
                    );

                    ui.add_space(5.0);

                    // 置顶按钮
                    let (pin_bg_color, pin_text_color) = if app.always_on_top {
                        (egui::Color32::from_rgb(60, 150, 60), egui::Color32::WHITE)
                    } else {
                        (egui::Color32::from_rgb(60, 60, 60), egui::Color32::GRAY)
                    };
                    let pin_text = egui::RichText::new("置顶").color(pin_text_color);
                    if ui
                        .add_sized([30.0, 28.0], egui::Button::new(pin_text).fill(pin_bg_color))
                        .clicked()
                    {
                        app.always_on_top = !app.always_on_top;
                        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                            if app.always_on_top {
                                egui::WindowLevel::AlwaysOnTop
                            } else {
                                egui::WindowLevel::Normal
                            },
                        ));
                    }

                    ui.add_space(10.0);

                    // 第一个任务卡片
                    ui.allocate_ui_with_layout(
                        egui::vec2(120.0, 50.0),
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| {
                            let status_text = if task.is_expired {
                                "执行中"
                            } else {
                                "待执行"
                            };
                            let status_color = if task.is_expired {
                                egui::Color32::from_rgb(100, 200, 100)
                            } else {
                                egui::Color32::from_rgb(150, 150, 255)
                            };
                            ui.label(
                                egui::RichText::new(status_text)
                                    .size(10.0)
                                    .color(status_color),
                            );
                            ui.label(egui::RichText::new(&task.current_name).size(15.0).strong());
                            ui.label(
                                egui::RichText::new(
                                    task.current_time.format("%m-%d %H:%M").to_string(),
                                )
                                .size(9.0)
                                .color(egui::Color32::GRAY),
                            );
                        },
                    );

                    ui.add_space(15.0);

                    // 第一个任务的倒计时
                    let countdown_color = theme::get_countdown_color(
                        task.remaining_seconds,
                        task.is_expired,
                        time_secs,
                    );
                    ui.label(
                        egui::RichText::new(&task.current_countdown)
                            .size(52.0)
                            .strong()
                            .color(countdown_color),
                    );

                    // 下一个任务
                    if let Some(ref next) = task.next_task {
                        ui.add_space(20.0);
                        ui.separator();
                        ui.add_space(20.0);

                        ui.allocate_ui_with_layout(
                            egui::vec2(120.0, 50.0),
                            egui::Layout::top_down(egui::Align::LEFT),
                            |ui| {
                                ui.label(
                                    egui::RichText::new("下一个")
                                        .size(10.0)
                                        .color(egui::Color32::from_rgb(150, 150, 255)),
                                );
                                ui.label(egui::RichText::new(&next.name).size(13.0).strong());
                                ui.label(
                                    egui::RichText::new(
                                        next.target_time.format("%m-%d %H:%M").to_string(),
                                    )
                                    .size(9.0)
                                    .color(egui::Color32::GRAY),
                                );
                            },
                        );

                        ui.add_space(15.0);

                        ui.label(
                            egui::RichText::new(&next.countdown)
                                .size(26.0)
                                .strong()
                                .color(egui::Color32::from_rgb(255, 100, 100)),
                        );
                    }

                    ui.add_space(10.0);
                });
            } else {
                // 加载中
                ui.horizontal_centered(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("正在加载任务数据...")
                            .size(12.0)
                            .color(egui::Color32::GRAY),
                    );
                });
            }
        }
    });
}

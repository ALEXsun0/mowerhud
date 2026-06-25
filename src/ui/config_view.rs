//! 配置界面：填写地址 / Basic 认证，测试连接并保存启动。

use crate::app::{AppState, MowerHUD};

pub fn show(app: &mut MowerHUD, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();

    egui::CentralPanel::default().show_inside(ui, |ui| {
        let left_margin = 20.0;

        // 顶部控制栏（可拖动）
        let title_bar_rect = {
            let mut rect = ui.available_rect_before_wrap();
            rect.set_height(30.0);
            rect
        };

        let title_bar_response = ui.allocate_rect(title_bar_rect, egui::Sense::click_and_drag());

        // egui 已判定为拖拽，直接发起系统窗口拖动
        if title_bar_response.dragged() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        // 在拖动区域上绘制控制栏内容
        ui.scope_builder(egui::UiBuilder::new().max_rect(title_bar_rect), |ui| {
            ui.horizontal(|ui| {
                ui.add_space(left_margin);
                ui.heading("MowerHUD");
                ui.add_space(5.0);
                ui.label(
                    egui::RichText::new("by A60")
                        .size(11.0)
                        .color(egui::Color32::GRAY),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(10.0);
                    // 关闭按钮
                    if ui.add_sized([30.0, 20.0], egui::Button::new("X")).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    // 最小化按钮
                    if ui.add_sized([30.0, 20.0], egui::Button::new("—")).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                });
            });
        });

        ui.add_space(8.0);

        // 标题区域
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("API配置").size(16.0).strong());
            ui.add_space(3.0);
            ui.label("请配置API连接信息");
        });

        ui.add_space(12.0);

        // 地址行
        ui.horizontal(|ui| {
            ui.add_space(left_margin);
            egui::Grid::new("main_config_grid")
                .num_columns(2)
                .spacing([15.0, 8.0])
                .striped(false)
                .show(ui, |ui| {
                    ui.label("地址:");
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        // 协议切换按钮
                        let protocol_text = if app.temp_use_https { "https" } else { "http" };
                        if ui
                            .add_sized([45.0, 18.0], egui::Button::new(protocol_text))
                            .clicked()
                        {
                            app.temp_use_https = !app.temp_use_https;
                        }
                        ui.label("://");
                        ui.add(
                            egui::TextEdit::singleline(&mut app.temp_address)
                                .hint_text("127.0.0.1 或 127.0.0.1:3289")
                                .desired_width(190.0),
                        );
                    });
                    ui.end_row();
                });
        });

        ui.add_space(10.0);

        // Basic 认证
        ui.horizontal(|ui| {
            ui.add_space(left_margin);
            ui.label("Basic认证（可选）");
        });

        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.add_space(left_margin);
            egui::Grid::new("auth_grid")
                .num_columns(2)
                .spacing([15.0, 8.0])
                .striped(false)
                .show(ui, |ui| {
                    ui.label("用户名:");
                    ui.add(egui::TextEdit::singleline(&mut app.temp_username).desired_width(258.0));
                    ui.end_row();

                    ui.label("密码:");
                    ui.add(
                        egui::TextEdit::singleline(&mut app.temp_password)
                            .password(true)
                            .desired_width(258.0),
                    );
                    ui.end_row();
                });
        });

        ui.add_space(10.0);

        // 提醒方式开关（通知 / 提示音相互独立）
        ui.horizontal(|ui| {
            ui.add_space(left_margin);
            ui.checkbox(&mut app.temp_notify_enabled, "启用通知");
            ui.add_space(15.0);
            ui.checkbox(&mut app.temp_sound_enabled, "启用提示音");
        });

        ui.add_space(12.0);

        // 连接状态显示
        let status = app.connection_status.lock().ok().map(|s| s.clone());
        if let Some(status) = status.filter(|s| !s.is_empty()) {
            ui.horizontal(|ui| {
                ui.add_space(left_margin);
                ui.label(status);
            });
            ui.add_space(6.0);
        }

        // 按钮区域
        ui.horizontal(|ui| {
            ui.add_space(left_margin);
            if ui
                .add_sized([95.0, 26.0], egui::Button::new("测试连接"))
                .clicked()
            {
                app.test_connection();
            }

            ui.add_space(15.0);

            if ui
                .add_sized([95.0, 26.0], egui::Button::new("保存并启动"))
                .clicked()
                && app.save_and_start()
            {
                app.state = AppState::Running;
                app.start_task_fetching();
            }
        });
    });
}

//! 隐身模式的系统托盘（仅 Windows）。
//!
//! 左键单击托盘 = 把 HUD 调回普通模式（重新显示）；右键菜单 = 状态 / 切换模式 / 退出。
//!
//! 关键：托盘图标**必须在一条自带消息循环的专用线程上创建**。tray-icon 在创建线程上建一个
//! 隐藏窗口，托盘交互产生的 `WM_USER_TRAYICON` 要由**该线程的消息泵**派发给 tray-icon 的窗口
//! 过程，事件才会进全局 channel。eframe/winit 的事件循环并不泵这个外部窗口的消息（实测托盘点
//! 击根本到不了代码），所以这里用专用线程跑 `GetMessage` 循环，与 winit 完全解耦。
//! 取到的事件转成 [`TrayCommand`] 入队，由主循环 `logic()` 取走执行。
//!
//! 菜单顶部有一行**禁用的状态项**，下面三个模式项用 `●/○` 标出当前选中的模式。两者都由本
//! 线程的 1 秒 `WM_TIMER` 定时从共享状态（app 每帧写入的状态文本 / 当前模式）刷新——这样隐身时
//! 右键托盘也能看到状态、任务、倒计时以及当前是哪个模式。

use crate::config::DisplayMode;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

/// 来自托盘的指令，交给 app 执行。
#[derive(Clone, Copy)]
pub enum TrayCommand {
    /// 切换到指定模式（隐身↔可见都由它完成）。
    SetMode(DisplayMode),
    /// 退出程序。
    Exit,
}

pub struct Tray {
    commands: Arc<Mutex<Vec<TrayCommand>>>,
}

/// 生成一个 32×32 的绿色圆点作为托盘图标。
fn make_icon() -> Option<Icon> {
    let (w, h) = (32u32, 32u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let (cx, cy) = (15.5f32, 15.5f32);
    let r2 = 15.0f32 * 15.0f32;
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            if dx * dx + dy * dy <= r2 {
                let i = ((y * w + x) * 4) as usize;
                rgba[i] = 100;
                rgba[i + 1] = 200;
                rgba[i + 2] = 100;
                rgba[i + 3] = 255;
            }
        }
    }
    Icon::from_rgba(rgba, w, h).ok()
}

/// 给模式项加 `●`(选中) / `○`(未选) 前缀。
fn mode_label(base: &str, active: bool) -> String {
    let dot = if active { "● " } else { "○ " };
    format!("{dot}{base}")
}

impl Tray {
    /// 在专用线程上创建托盘图标并跑消息循环。创建失败返回 None；此时 app 的 should_hide()
    /// 会拒绝隐身（退化为正常显示），以免窗口与托盘双失导致无法操作。
    /// - `ctx`：收到指令后尽力提前唤醒主循环（best-effort；最终由 logic() 定时链兜底取走）。
    /// - `status`：app 每帧写入的状态文本，本线程每秒读出刷新菜单顶部的状态项。
    /// - `mode`：app 每帧写入的当前模式（0=普通 1=单列 2=隐身），本线程据此标 `●`。
    pub fn new(
        ctx: egui::Context,
        status: Arc<Mutex<String>>,
        mode: Arc<AtomicU8>,
    ) -> Option<Tray> {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let queue = Arc::clone(&commands);
        // 专用线程同步回报创建结果，好让 should_hide()/调用方知道托盘是否可用。
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<bool>();

        std::thread::spawn(move || {
            // 必须在本线程创建：托盘窗口归本线程所有，其消息由下面的 GetMessage 循环来泵。
            let built = (|| {
                let menu = Menu::new();
                let status_item = MenuItem::new("加载中…", false, None); // 禁用 = 仅展示
                let normal = MenuItem::new("普通模式", true, None);
                let single = MenuItem::new("单列模式", true, None);
                let stealth = MenuItem::new("隐身模式", true, None);
                let exit = MenuItem::new("退出", true, None);
                menu.append(&status_item).ok()?;
                menu.append(&PredefinedMenuItem::separator()).ok()?;
                menu.append(&normal).ok()?;
                menu.append(&single).ok()?;
                menu.append(&stealth).ok()?;
                menu.append(&PredefinedMenuItem::separator()).ok()?;
                menu.append(&exit).ok()?;
                let tray = TrayIconBuilder::new()
                    .with_menu(Box::new(menu))
                    .with_tooltip("MowerHUD（左键显示，右键菜单）")
                    .with_icon(make_icon()?)
                    .build()
                    .ok()?;
                Some((tray, status_item, normal, single, stealth, exit))
            })();

            // _tray 及各菜单项句柄必须在循环期间保持存活。
            let (_tray, status_item, normal, single, stealth, exit) = match built {
                Some(v) => {
                    let _ = ready_tx.send(true);
                    v
                }
                None => {
                    let _ = ready_tx.send(false);
                    return;
                }
            };

            let tray_rx = TrayIconEvent::receiver().clone();
            let menu_rx = MenuEvent::receiver().clone();

            // 初始就按当前模式标好 ●，避免首秒没有任何标记。（0=普通 1=单列 2=隐身）
            let mut last_mode = mode.load(Ordering::Relaxed);
            normal.set_text(mode_label("普通模式", last_mode == 0));
            single.set_text(mode_label("单列模式", last_mode == 1));
            stealth.set_text(mode_label("隐身模式", last_mode == 2));

            // 本线程的消息循环：GetMessage 阻塞等消息 → 派发给 tray_proc（事件进 channel）→
            // 立刻把 channel 里的事件取出转成指令入队。右键菜单是模态的（TrackPopupMenu 在
            // DispatchMessage 内部跑完），菜单项点击的 MenuEvent 在派发返回后即可取到。
            // 另设 1 秒线程定时器，WM_TIMER 到点就刷新状态项文本与模式项的 ● 标记。
            unsafe {
                use winapi::um::winuser::{
                    DispatchMessageW, GetMessageW, MSG, SetTimer, TranslateMessage, WM_TIMER,
                };
                SetTimer(std::ptr::null_mut(), 1, 1000, None);
                let mut last_status = String::new();
                let mut msg: MSG = std::mem::zeroed();
                loop {
                    let r = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                    if r <= 0 {
                        break; // WM_QUIT 或错误
                    }

                    if msg.message == WM_TIMER {
                        if let Ok(s) = status.lock()
                            && *s != last_status
                        {
                            last_status.clone_from(&s);
                            status_item.set_text(&*s);
                        }
                        let m = mode.load(Ordering::Relaxed);
                        if m != last_mode {
                            last_mode = m;
                            normal.set_text(mode_label("普通模式", m == 0));
                            single.set_text(mode_label("单列模式", m == 1));
                            stealth.set_text(mode_label("隐身模式", m == 2));
                        }
                        continue;
                    }

                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);

                    let mut pushed = false;
                    while let Ok(ev) = tray_rx.try_recv() {
                        // 左键单击 = 调回普通模式（显示）
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = ev
                        {
                            queue
                                .lock()
                                .unwrap()
                                .push(TrayCommand::SetMode(DisplayMode::Normal));
                            pushed = true;
                        }
                    }
                    while let Ok(ev) = menu_rx.try_recv() {
                        let cmd = if ev.id == *normal.id() {
                            Some(TrayCommand::SetMode(DisplayMode::Normal))
                        } else if ev.id == *single.id() {
                            Some(TrayCommand::SetMode(DisplayMode::NormalSingle))
                        } else if ev.id == *stealth.id() {
                            Some(TrayCommand::SetMode(DisplayMode::Stealth))
                        } else if ev.id == *exit.id() {
                            Some(TrayCommand::Exit)
                        } else {
                            None
                        };
                        if let Some(c) = cmd {
                            queue.lock().unwrap().push(c);
                            pushed = true;
                        }
                    }
                    if pushed {
                        // 尽力提前唤醒主循环；最终由 logic() 的 request_repaint_after 定时链兜底。
                        ctx.request_repaint();
                    }
                }
            }
        });

        match ready_rx.recv() {
            Ok(true) => Some(Tray { commands }),
            _ => None,
        }
    }

    /// 取走并清空待处理指令。
    pub fn drain(&self) -> Vec<TrayCommand> {
        self.commands
            .lock()
            .map(|mut v| std::mem::take(&mut *v))
            .unwrap_or_default()
    }
}

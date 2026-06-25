//! 中文字体加载。

use egui::{Context, FontData, FontDefinitions, FontFamily};
use std::fs::read;
use std::sync::Arc;

/// 加载系统中文字体并设为首选，确保中文正常显示。
pub fn setup_chinese_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();

    // 各平台常见中文字体路径，按优先级尝试，命中一个即可
    let font_paths = [
        // Windows
        "C:\\Windows\\Fonts\\msyh.ttc",   // 微软雅黑
        "C:\\Windows\\Fonts\\simhei.ttf", // 黑体
        "C:\\Windows\\Fonts\\simsun.ttc", // 宋体
        // macOS
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/Library/Fonts/Arial Unicode.ttf",
        // Linux（Noto / 文泉驿等）
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        "/usr/share/fonts/truetype/arphic/uming.ttc",
    ];

    for font_path in font_paths {
        if let Ok(font_data) = read(font_path) {
            fonts.font_data.insert(
                "chinese_font".to_owned(),
                Arc::new(FontData::from_owned(font_data)),
            );

            // 插到比例字体家族最前面，优先使用
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "chinese_font".to_owned());

            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .push("chinese_font".to_owned());

            break; // 成功加载一个即可
        }
    }

    ctx.set_fonts(fonts);
}

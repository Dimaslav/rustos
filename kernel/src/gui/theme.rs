//! Палитры светлой/тёмной темы.

use core::sync::atomic::{AtomicU8, Ordering};
use crate::framebuffer::Color;

#[derive(Clone, Copy)]
pub struct Palette {
    pub window_bg: Color,
    pub window_bg_alt: Color,
    pub title_active: Color,
    pub title_inactive: Color,
    pub text: Color,
    pub text_muted: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub danger: Color,
    pub list_bg: Color,
    pub field_bg: Color,
    pub border: Color,
    pub shadow: Color,
    pub wallpaper_top: Color,
    pub wallpaper_bottom: Color,
    pub taskbar_bg: Color,
    // --- визуальные эффекты ---
    /// Прозрачность панели задач (0–255). Меньше = прозрачнее.
    pub taskbar_alpha: u8,
    /// Прозрачность стартового меню.
    pub menu_alpha: u8,
    /// Прозрачность заголовка активного окна.
    pub title_alpha: u8,
    /// Радиус box blur под панелями. 0 = выключено.
    pub blur_radius: usize,
}

pub const DARK: Palette = Palette {
    window_bg:        Color { r: 32,  g: 34,  b: 40  },
    window_bg_alt:    Color { r: 45,  g: 48,  b: 56  },
    title_active:     Color { r: 90,  g: 140, b: 255 },
    title_inactive:   Color { r: 60,  g: 63,  b: 72  },
    text:             Color { r: 235, g: 238, b: 245 },
    text_muted:       Color { r: 140, g: 145, b: 160 },
    accent:           Color { r: 90,  g: 140, b: 255 },
    accent_hover:     Color { r: 120, g: 165, b: 255 },
    danger:           Color { r: 232, g: 68,  b: 68  },
    list_bg:          Color { r: 37,  g: 40,  b: 48  },
    field_bg:         Color { r: 45,  g: 48,  b: 56  },
    border:           Color { r: 70,  g: 75,  b: 90  },
    shadow:           Color { r: 8,   g: 10,  b: 14  },
    wallpaper_top:    Color { r: 18,  g: 22,  b: 34  },
    wallpaper_bottom: Color { r: 40,  g: 30,  b: 60  },
    taskbar_bg:       Color { r: 22,  g: 24,  b: 32  },
    taskbar_alpha:    200,
    menu_alpha:       220,
    title_alpha:      235,
    blur_radius:      6,
};

pub const LIGHT: Palette = Palette {
    window_bg:        Color { r: 245, g: 246, b: 250 },
    window_bg_alt:    Color { r: 228, g: 231, b: 238 },
    title_active:     Color { r: 60,  g: 110, b: 220 },
    title_inactive:   Color { r: 200, g: 205, b: 215 },
    text:             Color { r: 24,  g: 26,  b: 32  },
    text_muted:       Color { r: 110, g: 115, b: 130 },
    accent:           Color { r: 60,  g: 110, b: 220 },
    accent_hover:     Color { r: 90,  g: 140, b: 240 },
    danger:           Color { r: 200, g: 50,  b: 50  },
    list_bg:          Color { r: 250, g: 251, b: 253 },
    field_bg:         Color { r: 255, g: 255, b: 255 },
    border:           Color { r: 190, g: 195, b: 205 },
    shadow:           Color { r: 170, g: 175, b: 185 },
    wallpaper_top:    Color { r: 205, g: 215, b: 240 },
    wallpaper_bottom: Color { r: 230, g: 220, b: 245 },
    taskbar_bg:       Color { r: 235, g: 237, b: 242 },
    taskbar_alpha:    215,
    menu_alpha:       230,
    title_alpha:      240,
    blur_radius:      6,
};

static THEME: AtomicU8 = AtomicU8::new(0);

#[inline]
pub fn palette() -> &'static Palette {
    match THEME.load(Ordering::Relaxed) {
        1 => &LIGHT,
        _ => &DARK,
    }
}

pub fn is_dark() -> bool {
    THEME.load(Ordering::Relaxed) == 0
}

pub fn toggle() {
    let next = if THEME.load(Ordering::Relaxed) == 0 { 1 } else { 0 };
    THEME.store(next, Ordering::Relaxed);
}

pub fn set_dark(dark: bool) {
    THEME.store(if dark { 0 } else { 1 }, Ordering::Relaxed);
}
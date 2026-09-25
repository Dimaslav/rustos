//! Задача автоповтора удерживаемой клавиши.

use super::sleep::Sleep;
use crate::keyboard;

/// Раз в ~55 мс проверяет, удерживается ли клавиша.
/// Если да — и с момента нажатия прошло >= ~500 мс — и с момента
/// предыдущего повтора прошло >= ~165 мс — пушит повтор.
pub async fn repeat_task() {
    let mut last_repeat: u64 = 0;
    loop {
        Sleep::new_ms(50).await;
        if let Some((key, start)) = keyboard::held() {
            let now = crate::interrupts::ticks();
            // START_DELAY = 9 тиков (~500 мс), INTERVAL = 3 тика (~165 мс)
            if now.saturating_sub(start) >= 9 && now.saturating_sub(last_repeat) >= 3 {
                keyboard::push_key(key);
                last_repeat = now;
            }
        } else {
            last_repeat = 0;
        }
    }
}
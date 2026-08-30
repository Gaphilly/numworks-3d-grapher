#[derive(Clone, Copy)]
#[repr(C)]
pub struct Color {
    pub rgb565: u16,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Point {
    pub x: u16,
    pub y: u16,
}

pub mod backlight {
    pub fn set_brightness(brightness: u8) {
        unsafe {
            eadk_backlight_set_brightness(brightness);
        }
    }
    pub fn brightness() -> u8 {
        unsafe {
            return eadk_backlight_brightness();
        }
    }

    extern "C" {
        fn eadk_backlight_set_brightness(brightness: u8);
        fn eadk_backlight_brightness() -> u8;
    }
}

pub mod display {
    use super::Color;
    use super::Point;
    use super::Rect;

    pub fn push_rect(rect: Rect, pixels: &[Color]) {
        let required = rect.width as usize * rect.height as usize;
        if pixels.len() < required {
            return;
        }
        unsafe {
            eadk_display_push_rect(rect, pixels.as_ptr());
        }
    }

    pub fn push_rect_uniform(rect: Rect, color: Color) {
        unsafe {
            eadk_display_push_rect_uniform(rect, color);
        }
    }

    pub fn wait_for_vblank() {
        unsafe {
            eadk_display_wait_for_vblank();
        }
    }

    pub fn draw_string(
        text: &[u8],
        point: Point,
        large_font: bool,
        text_color: Color,
        background_color: Color,
    ) {
        if text.last().copied() != Some(0) {
            return;
        }
        unsafe {
            eadk_display_draw_string(
                text.as_ptr(),
                point,
                large_font,
                text_color,
                background_color,
            );
        }
    }

    extern "C" {
        fn eadk_display_push_rect_uniform(rect: Rect, color: Color);
        fn eadk_display_push_rect(rect: Rect, color: *const Color);
        fn eadk_display_wait_for_vblank();
        fn eadk_display_draw_string(
            text: *const u8,
            point: Point,
            large_font: bool,
            text_color: Color,
            background_color: Color,
        );
    }
}

pub mod keyboard {
    pub type State = u64;

    pub const LEFT: u8 = 0;
    pub const UP: u8 = 1;
    pub const DOWN: u8 = 2;
    pub const RIGHT: u8 = 3;
    pub const OK: u8 = 4;
    pub const BACK: u8 = 5;
    pub const SHIFT: u8 = 12;
    pub const ALPHA: u8 = 13;
    pub const BACKSPACE: u8 = 17;
    pub const PLUS: u8 = 45;
    pub const MINUS: u8 = 46;

    pub fn scan() -> State {
        unsafe { eadk_keyboard_scan() }
    }

    pub fn key_down(state: State, key: u8) -> bool {
        key < 64 && ((state >> key) & 1) != 0
    }

    extern "C" {
        fn eadk_keyboard_scan() -> State;
    }
}

pub mod event {
    pub type Event = u16;

    pub const LEFT: Event = 0;
    pub const RIGHT: Event = 3;
    pub const OK: Event = 4;
    pub const BACK: Event = 5;
    pub const SHIFT: Event = 12;
    pub const ALPHA: Event = 13;
    pub const XNT: Event = 14;
    pub const TOOLBOX: Event = 16;
    pub const BACKSPACE: Event = 17;
    pub const COMMA: Event = 22;
    pub const POWER: Event = 23;
    pub const SINE: Event = 24;
    pub const COSINE: Event = 25;
    pub const TANGENT: Event = 26;
    pub const SQRT: Event = 28;
    pub const SQUARE: Event = 29;
    pub const SEVEN: Event = 30;
    pub const EIGHT: Event = 31;
    pub const NINE: Event = 32;
    pub const LEFT_PARENTHESIS: Event = 33;
    pub const RIGHT_PARENTHESIS: Event = 34;
    pub const FOUR: Event = 36;
    pub const FIVE: Event = 37;
    pub const SIX: Event = 38;
    pub const MULTIPLICATION: Event = 39;
    pub const DIVISION: Event = 40;
    pub const ONE: Event = 42;
    pub const TWO: Event = 43;
    pub const THREE: Event = 44;
    pub const PLUS: Event = 45;
    pub const MINUS: Event = 46;
    pub const ZERO: Event = 48;
    pub const DOT: Event = 49;
    pub const EE: Event = 50;
    pub const EXE: Event = 52;
    pub const SHIFT_LEFT: Event = 54;
    pub const SHIFT_RIGHT: Event = 57;
    pub const CLEAR: Event = 71;
    pub const SPACE: Event = 154;
    const NONE: Event = 216;
    const IDLE: Event = 223;

    pub fn poll() -> Option<Event> {
        // Epsilon's event implementation requires a timeout greater than
        // its 200 ms initial repeat delay. Calls are made only after a raw
        // key-down edge, so a queued event normally returns immediately.
        let mut timeout = 250_i32;
        let value = unsafe { eadk_event_get(&mut timeout) };
        if value == NONE || value == IDLE {
            None
        } else {
            Some(value)
        }
    }

    pub fn lowercase_letter(value: Event) -> Option<u8> {
        match value {
            126 => Some(b'a'),
            127 => Some(b'b'),
            128 => Some(b'c'),
            129 => Some(b'd'),
            130 => Some(b'e'),
            131 => Some(b'f'),
            132 => Some(b'g'),
            133 => Some(b'h'),
            134 => Some(b'i'),
            135 => Some(b'j'),
            136 => Some(b'k'),
            137 => Some(b'l'),
            138 => Some(b'm'),
            139 => Some(b'n'),
            140 => Some(b'o'),
            141 => Some(b'p'),
            142 => Some(b'q'),
            144 => Some(b'r'),
            145 => Some(b's'),
            146 => Some(b't'),
            147 => Some(b'u'),
            148 => Some(b'v'),
            150 => Some(b'w'),
            151 => Some(b'x'),
            152 => Some(b'y'),
            153 => Some(b'z'),
            180 => Some(b'a'),
            181 => Some(b'b'),
            182 => Some(b'c'),
            183 => Some(b'd'),
            184 => Some(b'e'),
            185 => Some(b'f'),
            186 => Some(b'g'),
            187 => Some(b'h'),
            188 => Some(b'i'),
            189 => Some(b'j'),
            190 => Some(b'k'),
            191 => Some(b'l'),
            192 => Some(b'm'),
            193 => Some(b'n'),
            194 => Some(b'o'),
            195 => Some(b'p'),
            196 => Some(b'q'),
            198 => Some(b'r'),
            199 => Some(b's'),
            200 => Some(b't'),
            201 => Some(b'u'),
            202 => Some(b'v'),
            204 => Some(b'w'),
            205 => Some(b'x'),
            206 => Some(b'y'),
            207 => Some(b'z'),
            _ => None,
        }
    }

    extern "C" {
        fn eadk_event_get(timeout: *mut i32) -> Event;
    }
}

pub mod timing {
    pub fn usleep(us: u32) {
        unsafe {
            eadk_timing_usleep(us);
        }
    }

    pub fn msleep(ms: u32) {
        unsafe {
            eadk_timing_msleep(ms);
        }
    }

    pub fn millis() -> u64 {
        unsafe {
            return eadk_timing_millis();
        }
    }

    extern "C" {
        fn eadk_timing_usleep(us: u32);
        fn eadk_timing_msleep(us: u32);
        fn eadk_timing_millis() -> u64;
    }
}

pub fn random() -> u32 {
    unsafe { return eadk_random() }
}

extern "C" {
    fn eadk_random() -> u32;
}

#[cfg(not(test))]
use core::panic::PanicInfo;

#[cfg(not(test))]
#[panic_handler]
fn panic(_panic: &PanicInfo<'_>) -> ! {
    loop {} // FIXME: Do something better. Exit the app maybe?
}

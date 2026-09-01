//! Minimal Rust bindings to the firmware-provided EADK ABI.
//!
//! These symbols are not implemented by this crate. `cargo build` intentionally
//! leaves them unresolved in a relocatable ARM object; nwlink resolves imports to
//! firmware trampolines while producing/installing the NWA. Consequently, a safe-
//! looking wrapper is not proof that the firmware call is intrinsically safe.
//! Callers must still satisfy pointer lifetime, buffer length, coordinate, string
//! termination, and ABI-version requirements.
//!
//! `#[repr(C)]` keeps aggregate field order/layout compatible with `eadk.h`.
//! Display coordinates are unsigned pixels from the top-left: x grows right and
//! y grows down on a 320×240 screen. Colors are packed RGB565 (5 red, 6 green,
//! 5 blue bits) in a `u16`.

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
/// One firmware-compatible RGB565 pixel.
pub struct Color {
    /// Packed 16-bit red/green/blue value.
    pub rgb565: u16,
}

#[derive(Clone, Copy)]
#[repr(C)]
/// Firmware display rectangle with top-left origin and unsigned dimensions.
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Copy)]
#[repr(C)]
/// Firmware display point in top-left-origin screen coordinates.
pub struct Point {
    pub x: u16,
    pub y: u16,
}

/// Backlight firmware calls. Currently unused by the grapher.
pub mod backlight {
    /// Requests a firmware brightness value.
    pub fn set_brightness(brightness: u8) {
        unsafe {
            eadk_backlight_set_brightness(brightness);
        }
    }
    /// Reads the current firmware brightness value.
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

/// Direct display operations supplied by EADK.
pub mod display {
    use super::Color;
    use super::Point;
    use super::Rect;

    /// Transfers `width * height` RGB565 pixels in row-major order.
    ///
    /// The wrapper rejects undersized slices before FFI. It does not prove that a
    /// rectangle is physically on-screen; callers must provide valid coordinates.
    pub fn push_rect(rect: Rect, pixels: &[Color]) {
        let required = rect.width as usize * rect.height as usize;
        if pixels.len() < required {
            return;
        }
        unsafe {
            eadk_display_push_rect(rect, pixels.as_ptr());
        }
    }

    /// Fills a firmware rectangle with one RGB565 value.
    pub fn push_rect_uniform(rect: Rect, color: Color) {
        unsafe {
            eadk_display_push_rect_uniform(rect, color);
        }
    }

    /// Synchronizes the start of a complete graph redraw with display refresh.
    pub fn wait_for_vblank() {
        unsafe {
            let _ = eadk_display_wait_for_vblank();
        }
    }

    /// Draws a NUL-terminated byte string using the firmware font.
    ///
    /// This checks termination before passing a pointer. Graph-coordinate labels
    /// must not use this function because they need to participate in band-buffer
    /// composition; it is reserved for independently dirtied UI regions.
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
        fn eadk_display_wait_for_vblank() -> bool;
        fn eadk_display_draw_string(
            text: *const u8,
            point: Point,
            large_font: bool,
            text_color: Color,
            background_color: Color,
        );
    }
}

/// Raw physical-key state from the NumWorks keyboard matrix.
pub mod keyboard {
    /// Bit mask where bit `eadk_key_t` is one while that physical key is held.
    pub type State = u64;

    // Values mirror the nwlink 0.0.19 EADK header exactly. Do not renumber these
    // as if they were ASCII or PC scan codes.
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
    pub const EXE: u8 = 52;

    /// Samples all physical keys without semantic Shift/Alpha translation.
    pub fn scan() -> State {
        unsafe { eadk_keyboard_scan() }
    }

    /// Bounds-checked test for one physical-key bit.
    pub fn key_down(state: State, key: u8) -> bool {
        key < 64 && ((state >> key) & 1) != 0
    }

    extern "C" {
        fn eadk_keyboard_scan() -> State;
    }
}

/// Semantic calculator events generated by Epsilon/EADK.
pub mod event {
    /// Firmware event identifier. These include modifier-generated characters and
    /// shortcuts that do not map one-to-one onto a physical key bit.
    pub type Event = u16;

    // Values mirror the installed EADK header. Letter ranges contain deliberate
    // gaps, hence `lowercase_letter` uses explicit matching rather than arithmetic.
    pub const LEFT: Event = 0;
    pub const UP: Event = 1;
    pub const DOWN: Event = 2;
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

    /// Polls for one semantic event with a bounded 250 ms firmware timeout.
    ///
    /// `eadk_event_get` takes a mutable timeout pointer and can wait until an
    /// event or timeout. The application calls this only after a raw down edge and
    /// only in tab/editor contexts; it is never an editor-owned blocking loop and
    /// never drives continuous camera motion. Epsilon's repeat machinery requires
    /// a budget beyond its ~200 ms initial delay, while an already pending event
    /// normally returns immediately.
    pub fn poll() -> Option<Event> {
        let mut timeout = 250_i32;
        let value = unsafe { eadk_event_get(&mut timeout) };
        if value == NONE || value == IDLE {
            None
        } else {
            Some(value)
        }
    }

    /// Converts supported Alpha-generated upper/lower letter events to the
    /// editor's lowercase ASCII expression language.
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

/// Firmware monotonic time and cooperative sleep calls.
pub mod timing {
    /// Sleeps for at least the requested microseconds according to firmware.
    pub fn usleep(us: u32) {
        unsafe {
            eadk_timing_usleep(us);
        }
    }

    /// Sleeps for at least the requested milliseconds according to firmware.
    pub fn msleep(ms: u32) {
        unsafe {
            eadk_timing_msleep(ms);
        }
    }

    /// Returns firmware monotonic milliseconds used for editor repeat timing.
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

/// Returns a firmware-provided pseudorandom value. Currently unused.
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
    // There is no terminal, unwinder, or useful recovery path in the external app
    // ABI. Production code therefore validates capacities/indices before FFI and
    // treats reaching this handler as an unrecoverable programming error.
    loop {}
}

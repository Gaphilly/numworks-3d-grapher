use crate::eadk::{self, Color, Point, Rect};

pub const HEADER_HEIGHT: u16 = 24;
const SCREEN_WIDTH: u16 = 320;
const SCREEN_HEIGHT: u16 = 240;
const TAB_WIDTHS: [u16; 3] = [106, 107, 107];
const TAB_X: [u16; 3] = [0, 106, 213];

const WHITE: Color = Color { rgb565: 0xffff };
const BLACK: Color = Color { rgb565: 0x0000 };
const BLUE: Color = Color { rgb565: 0x245f };
const LIGHT_BLUE: Color = Color { rgb565: 0x9d7f };
const LIGHT_GRAY: Color = Color { rgb565: 0xd69a };
const DARK_GRAY: Color = Color { rgb565: 0x630c };
const ORANGE: Color = Color { rgb565: 0xfd20 };

const TAB_LABELS: [&[u8]; 3] = [b"Graph\0", b"Equation\0", b"Settings\0"];
const TAB_TEXT_X: [u16; 3] = [31, 126, 235];

pub fn draw_header(active: usize, selected: usize, tabs_focused: bool) {
    let mut index = 0;
    while index < TAB_LABELS.len() {
        let is_active = index == active;
        let is_selected = tabs_focused && index == selected;
        let background = if is_selected {
            LIGHT_BLUE
        } else if is_active {
            BLUE
        } else {
            LIGHT_GRAY
        };
        let text = if is_active { WHITE } else { BLACK };
        eadk::display::push_rect_uniform(
            Rect {
                x: TAB_X[index],
                y: 0,
                width: TAB_WIDTHS[index],
                height: HEADER_HEIGHT,
            },
            background,
        );
        eadk::display::draw_string(
            TAB_LABELS[index],
            Point {
                x: TAB_TEXT_X[index],
                y: 4,
            },
            false,
            text,
            background,
        );
        index += 1;
    }

    let indicator_tab = if tabs_focused { selected } else { active };
    eadk::display::push_rect_uniform(
        Rect {
            x: TAB_X[indicator_tab],
            y: HEADER_HEIGHT - 3,
            width: TAB_WIDTHS[indicator_tab],
            height: 3,
        },
        if tabs_focused { ORANGE } else { DARK_GRAY },
    );
}

pub fn draw_equation_placeholder() {
    clear_content();
    draw_centered_message(b"Equation\0", 82, 105);
    draw_centered_message(b"Editor coming later\0", 73, 128);
}

pub fn draw_settings_placeholder() {
    clear_content();
    draw_centered_message(b"Settings\0", 79, 105);
    draw_centered_message(b"Options coming later\0", 69, 128);
}

fn clear_content() {
    eadk::display::push_rect_uniform(
        Rect {
            x: 0,
            y: HEADER_HEIGHT,
            width: SCREEN_WIDTH,
            height: SCREEN_HEIGHT - HEADER_HEIGHT,
        },
        WHITE,
    );
}

fn draw_centered_message(text: &[u8], x: u16, y: u16) {
    eadk::display::draw_string(text, Point { x, y }, false, DARK_GRAY, WHITE);
}

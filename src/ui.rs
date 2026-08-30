//! Lightweight tab header and non-graph content drawing using EADK primitives.
//!
//! These views are redrawn only when their corresponding dirty flag is set.
//! Direct firmware string drawing is appropriate here because each UI region is
//! completed synchronously and is not interleaved with graph bands. Graph labels
//! are different: they must remain bitmap glyphs inside `rendering`'s band buffer
//! to preserve composition order and avoid stale/flashing text.

use crate::eadk::{self, Color, Point, Rect};
use crate::editor::{EquationEditor, VISIBLE_CHARACTERS};
use crate::expression::ParseError;

/// Header height; the graph renderer owns the remaining 320×216 viewport.
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
const FIELD_BACKGROUND: Color = Color { rgb565: 0xffdf };

const TAB_LABELS: [&[u8]; 3] = [b"Graph\0", b"Equation\0", b"Settings\0"];
const TAB_TEXT_X: [u16; 3] = [31, 126, 235];

/// Draws all three tabs and the active/keyboard-focus indicator.
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

/// Redraws the Equation content region, fixed field, cursor, help, and parse error.
/// The stack NUL-terminated copy is required by EADK's C string ABI.
pub fn draw_equation_editor(editor: &EquationEditor, focused: bool) {
    clear_content();
    eadk::display::draw_string(
        b"Equation\0",
        Point { x: 12, y: 38 },
        false,
        DARK_GRAY,
        WHITE,
    );
    eadk::display::draw_string(b"f(x,y) =\0", Point { x: 12, y: 61 }, false, BLACK, WHITE);

    let border = if focused { BLUE } else { DARK_GRAY };
    eadk::display::push_rect_uniform(
        Rect {
            x: 10,
            y: 82,
            width: 300,
            height: 20,
        },
        FIELD_BACKGROUND,
    );
    eadk::display::push_rect_uniform(
        Rect {
            x: 9,
            y: 81,
            width: 302,
            height: 1,
        },
        border,
    );
    eadk::display::push_rect_uniform(
        Rect {
            x: 9,
            y: 81,
            width: 1,
            height: 23,
        },
        border,
    );
    eadk::display::push_rect_uniform(
        Rect {
            x: 310,
            y: 81,
            width: 1,
            height: 23,
        },
        border,
    );
    eadk::display::push_rect_uniform(
        Rect {
            x: 9,
            y: 102,
            width: 302,
            height: 2,
        },
        border,
    );

    let mut visible = [0_u8; VISIBLE_CHARACTERS + 1];
    let source = editor.visible_bytes();
    let mut index = 0;
    while index < source.len() && index < VISIBLE_CHARACTERS {
        visible[index] = source[index];
        index += 1;
    }
    eadk::display::draw_string(
        &visible,
        Point { x: 12, y: 85 },
        false,
        BLACK,
        FIELD_BACKGROUND,
    );

    if focused {
        let cursor_column = editor.cursor() - editor.scroll();
        eadk::display::push_rect_uniform(
            Rect {
                x: 12 + cursor_column as u16 * 7,
                y: 99,
                width: 6,
                height: 2,
            },
            BLUE,
        );
    }

    if let Some(error) = editor.error() {
        eadk::display::draw_string(
            error_message(error),
            Point { x: 12, y: 121 },
            false,
            Color { rgb565: 0xb800 },
            WHITE,
        );
    } else {
        eadk::display::draw_string(
            b"EXE: graph   OK: tabs\0",
            Point { x: 12, y: 121 },
            false,
            DARK_GRAY,
            WHITE,
        );
    }
    eadk::display::draw_string(
        b"Back: cancel   Shift+BS: clear\0",
        Point { x: 12, y: 145 },
        false,
        DARK_GRAY,
        WHITE,
    );
    eadk::display::draw_string(
        b"Toolbox: abs()\0",
        Point { x: 12, y: 169 },
        false,
        DARK_GRAY,
        WHITE,
    );
}

/// Draws the allocation-free Settings placeholder until settings are implemented.
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

fn error_message(error: ParseError) -> &'static [u8] {
    match error {
        ParseError::InvalidCharacter => b"Invalid character\0",
        ParseError::UnknownFunction => b"Unknown function\0",
        ParseError::MissingClosingParenthesis => b"Missing ')'\0",
        ParseError::MissingOpeningParenthesis => b"Missing '('\0",
        ParseError::ExpressionTooLong => b"Expression too long\0",
        ParseError::BytecodeTooLarge
        | ParseError::OperatorStackOverflow
        | ParseError::EvaluationStackOverflow => b"Expression too complex\0",
        ParseError::EmptyExpression => b"Enter an expression\0",
        ParseError::InvalidNumber => b"Invalid number\0",
        ParseError::MissingOperand | ParseError::MissingOperator => b"Invalid expression\0",
    }
}

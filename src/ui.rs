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
use crate::graph::{GraphOptions, LightingPreset, RenderingMode, SurfacePalette};
use crate::settings::{
    AppearanceItem, DomainField, NumberText, NumericError, SettingsItem, SettingsPage,
    SettingsState, NUMERIC_VISIBLE_CHARACTERS,
};
use crate::surface::{Domain, DomainError};

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

// This user-facing version is intentionally maintained by hand. Do not derive,
// synchronize, or update it from Cargo metadata, Git tags, or release tooling;
// change it only when the project owner explicitly requests a displayed update.
const APPLICATION_DISPLAY_VERSION: &[u8] = b"v2.3.0\0";
const SMALL_FONT_CHARACTER_WIDTH: u16 = 7;
const VERSION_TEXT_WIDTH: u16 =
    (APPLICATION_DISPLAY_VERSION.len() as u16 - 1) * SMALL_FONT_CHARACTER_WIDTH;
const VERSION_TEXT_X: u16 = (SCREEN_WIDTH - VERSION_TEXT_WIDTH) / 2;
const VERSION_TEXT_Y: u16 = 218;
const PERFORMANCE_TEXT_Y: u16 = 204;

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

/// Draws the current allocation-free Settings page.
///
/// Main-menu selection and numeric drafts are state only; this function performs
/// no mutation. As with Equation, firmware strings are safe here because the
/// complete Settings content region is redrawn synchronously, never between graph
/// band transfers.
pub fn draw_settings(
    settings: &SettingsState,
    options: GraphOptions,
    domain: Domain,
    focused: bool,
    graph_render_ms: u32,
) {
    clear_content();
    match settings.page() {
        SettingsPage::Main => draw_settings_menu(settings, options, focused),
        SettingsPage::Domain => draw_domain_settings(settings, domain, focused),
        SettingsPage::Appearance => draw_appearance_settings(settings, options, focused),
    }
    eadk::display::draw_string(
        APPLICATION_DISPLAY_VERSION,
        Point {
            x: VERSION_TEXT_X,
            y: VERSION_TEXT_Y,
        },
        false,
        DARK_GRAY,
        WHITE,
    );
    if options.show_performance {
        let mut text = [0_u8; 24];
        let text = format_render_performance(graph_render_ms, &mut text);
        eadk::display::draw_string(
            text,
            Point {
                x: 4,
                y: PERFORMANCE_TEXT_Y,
            },
            false,
            DARK_GRAY,
            WHITE,
        );
    }
}

fn format_render_performance(value: u32, buffer: &mut [u8; 24]) -> &[u8] {
    let prefix = b"Last:";
    let mut index = 0;
    while index < prefix.len() {
        buffer[index] = prefix[index];
        index += 1;
    }
    let mut reverse = [0_u8; 10];
    let mut count = 0;
    let mut remaining = value;
    loop {
        reverse[count] = b'0' + (remaining % 10) as u8;
        count += 1;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    while count > 0 && index + 3 < buffer.len() {
        count -= 1;
        buffer[index] = reverse[count];
        index += 1;
    }
    buffer[index] = b'm';
    buffer[index + 1] = b's';
    index += 2;
    buffer[index] = b' ';
    index += 1;
    let fps = if value == 0 { 0 } else { 1000 / value };
    let mut fps_reverse = [0_u8; 10];
    let mut fps_count = 0;
    let mut remaining = fps;
    loop {
        fps_reverse[fps_count] = b'0' + (remaining % 10) as u8;
        fps_count += 1;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    while fps_count > 0 {
        fps_count -= 1;
        buffer[index] = fps_reverse[fps_count];
        index += 1;
    }
    buffer[index] = b'F';
    buffer[index + 1] = b'P';
    buffer[index + 2] = b'S';
    buffer[index + 3] = 0;
    &buffer[..index + 4]
}

const SETTINGS_ROW_TOP: u16 = 30;
const SETTINGS_ROW_HEIGHT: u16 = 22;
const SETTINGS_LABELS: [&[u8]; 8] = [
    b"Rendering\0",
    b"Ground grid\0",
    b"Axes\0",
    b"Ticks\0",
    b"Labels\0",
    b"Domain\0",
    b"Reset camera\0",
    b"Performance\0",
];

fn draw_settings_menu(settings: &SettingsState, options: GraphOptions, focused: bool) {
    let selected = settings.selected_item().index();
    let mut row = 0;
    while row < SETTINGS_LABELS.len() {
        let top = SETTINGS_ROW_TOP + row as u16 * SETTINGS_ROW_HEIGHT;
        let is_selected = row == selected;
        let background = if is_selected { FIELD_BACKGROUND } else { WHITE };
        eadk::display::push_rect_uniform(
            Rect {
                x: 8,
                y: top,
                width: 304,
                height: SETTINGS_ROW_HEIGHT - 2,
            },
            background,
        );
        if is_selected {
            eadk::display::push_rect_uniform(
                Rect {
                    x: 8,
                    y: top,
                    width: 3,
                    height: SETTINGS_ROW_HEIGHT - 2,
                },
                if focused { ORANGE } else { DARK_GRAY },
            );
        }
        eadk::display::draw_string(
            SETTINGS_LABELS[row],
            Point { x: 16, y: top + 2 },
            false,
            BLACK,
            background,
        );
        let (value, x) = setting_value(SettingsItem::from_index(row), options);
        eadk::display::draw_string(
            value,
            Point { x, y: top + 2 },
            false,
            if is_selected { BLUE } else { DARK_GRAY },
            background,
        );
        row += 1;
    }
}

fn setting_value(item: SettingsItem, options: GraphOptions) -> (&'static [u8], u16) {
    match item {
        SettingsItem::RenderingMode => match options.rendering_mode {
            RenderingMode::Wireframe => (b"Wireframe >\0", 214),
            RenderingMode::Solid => (b"Solid >\0", 256),
        },
        SettingsItem::GroundGrid => on_off(options.show_grid),
        SettingsItem::Axes => on_off(options.show_axes),
        SettingsItem::Ticks => on_off(options.show_ticks),
        SettingsItem::Labels => on_off(options.show_labels),
        SettingsItem::Performance => on_off(options.show_performance),
        SettingsItem::Domain => (b"EXE >\0", 270),
        SettingsItem::ResetCamera => (b"EXE\0", 284),
    }
}

const APPEARANCE_LABELS: [&[u8]; 2] = [b"Lighting\0", b"Surface color\0"];
const APPEARANCE_ROW_TOP: u16 = 56;
const APPEARANCE_ROW_HEIGHT: u16 = 34;

fn draw_appearance_settings(settings: &SettingsState, options: GraphOptions, focused: bool) {
    eadk::display::draw_string(
        b"Solid appearance\0",
        Point { x: 12, y: 31 },
        false,
        DARK_GRAY,
        WHITE,
    );
    let selected = settings.selected_appearance_item().index();
    let mut row = 0;
    while row < APPEARANCE_LABELS.len() {
        let top = APPEARANCE_ROW_TOP + row as u16 * APPEARANCE_ROW_HEIGHT;
        let is_selected = row == selected;
        let background = if is_selected { FIELD_BACKGROUND } else { WHITE };
        eadk::display::push_rect_uniform(
            Rect {
                x: 8,
                y: top,
                width: 304,
                height: APPEARANCE_ROW_HEIGHT - 4,
            },
            background,
        );
        if is_selected {
            eadk::display::push_rect_uniform(
                Rect {
                    x: 8,
                    y: top,
                    width: 3,
                    height: APPEARANCE_ROW_HEIGHT - 4,
                },
                if focused { ORANGE } else { DARK_GRAY },
            );
        }
        eadk::display::draw_string(
            APPEARANCE_LABELS[row],
            Point { x: 16, y: top + 7 },
            false,
            BLACK,
            background,
        );
        let (value, x) = appearance_value(AppearanceItem::from_index(row as u8), options);
        eadk::display::draw_string(
            value,
            Point { x, y: top + 7 },
            false,
            if is_selected { BLUE } else { DARK_GRAY },
            background,
        );
        row += 1;
    }
    eadk::display::draw_string(
        b"Left/Right: change   Back: settings\0",
        Point { x: 12, y: 145 },
        false,
        DARK_GRAY,
        WHITE,
    );
}

fn appearance_value(item: AppearanceItem, options: GraphOptions) -> (&'static [u8], u16) {
    match item {
        AppearanceItem::Lighting => match options.lighting {
            LightingPreset::Standard => (b"Standard\0", 242),
            LightingPreset::Soft => (b"Soft\0", 277),
            LightingPreset::Strong => (b"Strong\0", 263),
        },
        AppearanceItem::SurfaceColor => match options.surface_palette {
            SurfacePalette::Blue => (b"Blue\0", 277),
            SurfacePalette::Green => (b"Green\0", 270),
            SurfacePalette::Orange => (b"Orange\0", 263),
            SurfacePalette::Purple => (b"Purple\0", 263),
            SurfacePalette::Gray => (b"Gray\0", 277),
        },
    }
}

fn on_off(value: bool) -> (&'static [u8], u16) {
    if value {
        (b"On\0", 291)
    } else {
        (b"Off\0", 284)
    }
}

const DOMAIN_LABELS: [&[u8]; 4] = [b"Xmin\0", b"Xmax\0", b"Ymin\0", b"Ymax\0"];
const DOMAIN_TITLE_Y: u16 = 31;
const DOMAIN_ROW_TOP: u16 = 55;
const DOMAIN_ROW_HEIGHT: u16 = 30;
const DOMAIN_FIELD_X: u16 = 82;
const DOMAIN_FIELD_WIDTH: u16 = 225;

fn draw_domain_settings(settings: &SettingsState, domain: Domain, focused: bool) {
    eadk::display::draw_string(
        b"Graph domain\0",
        Point {
            x: 12,
            y: DOMAIN_TITLE_Y,
        },
        false,
        DARK_GRAY,
        WHITE,
    );
    let selected = settings.selected_domain_field().index();
    let mut row = 0;
    while row < DOMAIN_LABELS.len() {
        let top = DOMAIN_ROW_TOP + row as u16 * DOMAIN_ROW_HEIGHT;
        let is_selected = row == selected;
        eadk::display::draw_string(
            DOMAIN_LABELS[row],
            Point { x: 16, y: top + 3 },
            false,
            BLACK,
            WHITE,
        );
        draw_domain_field(
            settings,
            DomainField::from_index(row),
            domain,
            top,
            is_selected,
            focused,
        );
        row += 1;
    }

    let message = match settings.error() {
        Some(error) => numeric_error_message(error),
        None if settings.is_editing() => b"EXE: apply   Back: cancel\0",
        None => b"EXE: edit   Back: settings\0",
    };
    eadk::display::draw_string(
        message,
        Point { x: 12, y: 190 },
        false,
        if settings.error().is_some() {
            Color { rgb565: 0xb800 }
        } else {
            DARK_GRAY
        },
        WHITE,
    );
}

fn draw_domain_field(
    settings: &SettingsState,
    field: DomainField,
    domain: Domain,
    top: u16,
    selected: bool,
    focused: bool,
) {
    let editing = selected && settings.is_editing();
    let background = if selected { FIELD_BACKGROUND } else { WHITE };
    eadk::display::push_rect_uniform(
        Rect {
            x: DOMAIN_FIELD_X,
            y: top,
            width: DOMAIN_FIELD_WIDTH,
            height: 23,
        },
        background,
    );
    if selected {
        let border = if focused { BLUE } else { DARK_GRAY };
        draw_field_border(DOMAIN_FIELD_X, top, DOMAIN_FIELD_WIDTH, 23, border);
    }

    if editing {
        let mut visible = [0_u8; NUMERIC_VISIBLE_CHARACTERS + 1];
        let bytes = settings.edit_visible_bytes();
        let mut index = 0;
        while index < bytes.len() && index < NUMERIC_VISIBLE_CHARACTERS {
            visible[index] = bytes[index];
            index += 1;
        }
        eadk::display::draw_string(
            &visible,
            Point {
                x: DOMAIN_FIELD_X + 5,
                y: top + 3,
            },
            false,
            BLACK,
            background,
        );
        if focused {
            let cursor_column = settings.edit_cursor() - settings.edit_scroll();
            eadk::display::push_rect_uniform(
                Rect {
                    x: DOMAIN_FIELD_X + 5 + cursor_column as u16 * SMALL_FONT_CHARACTER_WIDTH,
                    y: top + 19,
                    width: 6,
                    height: 2,
                },
                BLUE,
            );
        }
    } else {
        let text = NumberText::new(field.value(domain));
        eadk::display::draw_string(
            text.as_c_string(),
            Point {
                x: DOMAIN_FIELD_X + 5,
                y: top + 3,
            },
            false,
            BLACK,
            background,
        );
    }
}

fn draw_field_border(x: u16, y: u16, width: u16, height: u16, color: Color) {
    eadk::display::push_rect_uniform(
        Rect {
            x,
            y,
            width,
            height: 1,
        },
        color,
    );
    eadk::display::push_rect_uniform(
        Rect {
            x,
            y,
            width: 1,
            height,
        },
        color,
    );
    eadk::display::push_rect_uniform(
        Rect {
            x: x + width - 1,
            y,
            width: 1,
            height,
        },
        color,
    );
    eadk::display::push_rect_uniform(
        Rect {
            x,
            y: y + height - 1,
            width,
            height: 1,
        },
        color,
    );
}

fn numeric_error_message(error: NumericError) -> &'static [u8] {
    match error {
        NumericError::InvalidNumber => b"Invalid number\0",
        NumericError::TooLong => b"Number too long\0",
        NumericError::Domain(DomainError::NonFinite) => b"Finite values only\0",
        NumericError::Domain(DomainError::Inverted) => b"Minimum must be below maximum\0",
        NumericError::Domain(DomainError::TooNarrow) => b"Domain is too narrow\0",
        NumericError::Domain(DomainError::TooLarge) => b"Domain is too large\0",
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displayed_release_version_remains_manually_fixed() {
        assert_eq!(APPLICATION_DISPLAY_VERSION, b"v2.3.0\0");
    }

    #[test]
    fn render_performance_text_is_bounded_and_c_terminated() {
        let mut buffer = [0_u8; 24];
        assert_eq!(
            format_render_performance(100, &mut buffer),
            b"Last:100ms 10FPS\0"
        );
        let text = format_render_performance(u32::MAX, &mut buffer);
        assert_eq!(text.last(), Some(&0));
        assert!(text.len() <= buffer.len());
    }
}

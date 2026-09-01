//! Fixed-capacity Equation text editor and non-blocking held-key repetition.
//!
//! The editor stores at most 96 single-byte expression characters, matching the
//! compiler limit. Cursor and horizontal scroll are byte indices; this is valid
//! because the accepted calculator mapping inserts ASCII only. The edited source
//! is intentionally independent of active compiled bytecode, so a parse failure
//! cannot destroy the graph that was last accepted with EXE.
//!
//! Calculator-style characters arrive as semantic EADK events. Backspace and
//! cursor keys additionally use raw-state timing for held-key repeat. Event polling
//! remains bounded and controlled by `main`; Equation focus must never enter a
//! blocking `eadk_event_get` loop because that would stall tab/focus transitions.

use crate::eadk::{event, keyboard};
use crate::expression::{CompiledExpression, ParseError, MAX_EXPRESSION_LENGTH};

/// Maximum number of fixed-width characters visible in the expression field.
pub const VISIBLE_CHARACTERS: usize = 40;
/// Delay before a supported held editing key begins repeating.
pub const REPEAT_INITIAL_DELAY_MS: u64 = 450;
/// Interval between repeated Backspace/Left/Right editor actions.
pub const REPEAT_INTERVAL_MS: u64 = 75;
const INITIAL_EXPRESSION: &[u8] = b"sin(x) * cos(y)";
/// Number of rows in each column of the compact Toolbox function picker.
pub const FUNCTION_PICKER_ROWS: usize = 8;

/// Function template selected by the Equation Toolbox picker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FunctionTemplate {
    Sin,
    Cos,
    Tan,
    Sqrt,
    Abs,
    Floor,
    Ceil,
    Round,
    Exp,
    Ln,
    Log,
    Min,
    Max,
    Asin,
    Acos,
    Atan,
}

impl FunctionTemplate {
    /// Resolves one of the fixed 2-by-8 picker positions.
    pub fn from_position(column: u8, row: u8) -> FunctionTemplate {
        match (column, row) {
            (0, 0) => FunctionTemplate::Sin,
            (0, 1) => FunctionTemplate::Cos,
            (0, 2) => FunctionTemplate::Tan,
            (0, 3) => FunctionTemplate::Sqrt,
            (0, 4) => FunctionTemplate::Abs,
            (0, 5) => FunctionTemplate::Floor,
            (0, 6) => FunctionTemplate::Ceil,
            (0, _) => FunctionTemplate::Round,
            (1, 0) => FunctionTemplate::Exp,
            (1, 1) => FunctionTemplate::Ln,
            (1, 2) => FunctionTemplate::Log,
            (1, 3) => FunctionTemplate::Min,
            (1, 4) => FunctionTemplate::Max,
            (1, 5) => FunctionTemplate::Asin,
            (1, 6) => FunctionTemplate::Acos,
            _ => FunctionTemplate::Atan,
        }
    }

    /// Display label and inserted ASCII template.
    pub fn source(self) -> &'static [u8] {
        match self {
            FunctionTemplate::Sin => b"sin()",
            FunctionTemplate::Cos => b"cos()",
            FunctionTemplate::Tan => b"tan()",
            FunctionTemplate::Sqrt => b"sqrt()",
            FunctionTemplate::Abs => b"abs()",
            FunctionTemplate::Floor => b"floor()",
            FunctionTemplate::Ceil => b"ceil()",
            FunctionTemplate::Round => b"round()",
            FunctionTemplate::Exp => b"exp()",
            FunctionTemplate::Ln => b"ln()",
            FunctionTemplate::Log => b"log(,)",
            FunctionTemplate::Min => b"min(,)",
            FunctionTemplate::Max => b"max(,)",
            FunctionTemplate::Asin => b"asin()",
            FunctionTemplate::Acos => b"acos()",
            FunctionTemplate::Atan => b"atan()",
        }
    }

    /// NUL-terminated label used directly by the EADK function-picker UI.
    pub fn label(self) -> &'static [u8] {
        match self {
            FunctionTemplate::Sin => b"sin\0",
            FunctionTemplate::Cos => b"cos\0",
            FunctionTemplate::Tan => b"tan\0",
            FunctionTemplate::Sqrt => b"sqrt\0",
            FunctionTemplate::Abs => b"abs\0",
            FunctionTemplate::Floor => b"floor\0",
            FunctionTemplate::Ceil => b"ceil\0",
            FunctionTemplate::Round => b"round\0",
            FunctionTemplate::Exp => b"exp\0",
            FunctionTemplate::Ln => b"ln\0",
            FunctionTemplate::Log => b"log\0",
            FunctionTemplate::Min => b"min\0",
            FunctionTemplate::Max => b"max\0",
            FunctionTemplate::Asin => b"asin\0",
            FunctionTemplate::Acos => b"acos\0",
            FunctionTemplate::Atan => b"atan\0",
        }
    }

    /// Binary templates place the cursor immediately after the opening `(`.
    pub fn is_binary(self) -> bool {
        matches!(
            self,
            FunctionTemplate::Log | FunctionTemplate::Min | FunctionTemplate::Max
        )
    }
}

#[derive(Clone, Copy, PartialEq)]
enum RepeatKey {
    None,
    Backspace,
    Left,
    Right,
}

/// Time/held-key state for editor-only application-level repetition.
pub struct EditorKeyRepeat {
    key: RepeatKey,
    next_repeat_ms: u64,
}

impl EditorKeyRepeat {
    /// Creates an idle repeat state.
    pub fn new() -> EditorKeyRepeat {
        EditorKeyRepeat {
            key: RepeatKey::None,
            next_repeat_ms: 0,
        }
    }

    /// Cancels any pending initial delay or repeat sequence.
    pub fn reset(&mut self) {
        self.key = RepeatKey::None;
        self.next_repeat_ms = 0;
    }

    /// Observes raw held keys and emits at most one semantic editing event.
    /// Modifiers disable repetition; OK, Back, EXE, and character insertion are
    /// deliberately never synthesized here.
    pub fn update(&mut self, keys: keyboard::State, now_ms: u64) -> Option<event::Event> {
        let modifier_down =
            keyboard::key_down(keys, keyboard::SHIFT) || keyboard::key_down(keys, keyboard::ALPHA);
        let held = if modifier_down {
            RepeatKey::None
        } else if keyboard::key_down(keys, keyboard::BACKSPACE) {
            RepeatKey::Backspace
        } else if keyboard::key_down(keys, keyboard::LEFT) {
            RepeatKey::Left
        } else if keyboard::key_down(keys, keyboard::RIGHT) {
            RepeatKey::Right
        } else {
            RepeatKey::None
        };

        if held == RepeatKey::None {
            self.reset();
            return None;
        }
        if held != self.key {
            self.key = held;
            self.next_repeat_ms = now_ms.saturating_add(REPEAT_INITIAL_DELAY_MS);
            return None;
        }
        if now_ms < self.next_repeat_ms {
            return None;
        }
        self.next_repeat_ms = now_ms.saturating_add(REPEAT_INTERVAL_MS);
        match held {
            RepeatKey::Backspace => Some(event::BACKSPACE),
            RepeatKey::Left => Some(event::LEFT),
            RepeatKey::Right => Some(event::RIGHT),
            RepeatKey::None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// Intent produced by one editor event for `AppState` to resolve.
pub enum EditorAction {
    None,
    Changed,
    Submit,
    Cancel,
    FocusTabs,
}

/// Editable source, cursor/scroll state, and the current parse error.
pub struct EquationEditor {
    buffer: [u8; MAX_EXPRESSION_LENGTH],
    length: usize,
    cursor: usize,
    scroll: usize,
    error: Option<ParseError>,
    picker_open: bool,
    picker_column: u8,
    picker_row: u8,
}

impl EquationEditor {
    /// Creates an editor preloaded with `sin(x) * cos(y)`.
    pub fn new() -> EquationEditor {
        let mut editor = EquationEditor {
            buffer: [0; MAX_EXPRESSION_LENGTH],
            length: 0,
            cursor: 0,
            scroll: 0,
            error: None,
            picker_open: false,
            picker_column: 0,
            picker_row: 0,
        };
        let _ = editor.insert_bytes(INITIAL_EXPRESSION);
        editor
    }

    /// Returns the current ASCII source as UTF-8 without allocating.
    pub fn source(&self) -> &str {
        match core::str::from_utf8(&self.buffer[..self.length]) {
            Ok(source) => source,
            Err(_) => "",
        }
    }

    /// Current insertion point in bytes/characters.
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    /// First visible byte in the horizontally scrolled field.
    pub fn scroll(&self) -> usize {
        self.scroll
    }
    /// Visible source slice; UI copies it into a stack NUL-terminated EADK buffer.
    pub fn visible_bytes(&self) -> &[u8] {
        let end = core::cmp::min(self.length, self.scroll + VISIBLE_CHARACTERS);
        &self.buffer[self.scroll..end]
    }
    /// Last compilation error, cleared by the next successful edit/compile.
    pub fn error(&self) -> Option<ParseError> {
        self.error
    }

    /// Whether the non-blocking Toolbox function picker owns Equation arrows.
    pub fn function_picker_open(&self) -> bool {
        self.picker_open
    }

    /// Selected fixed template while the picker is visible.
    pub fn selected_function_template(&self) -> FunctionTemplate {
        FunctionTemplate::from_position(self.picker_column, self.picker_row)
    }

    /// Selected picker column for the Equation UI.
    pub fn function_picker_column(&self) -> u8 {
        self.picker_column
    }

    /// Selected picker row for the Equation UI.
    pub fn function_picker_row(&self) -> u8 {
        self.picker_row
    }

    /// Closes the picker without changing source text or the active bytecode.
    pub fn close_function_picker(&mut self) -> bool {
        if !self.picker_open {
            return false;
        }
        self.picker_open = false;
        true
    }

    /// Maps a semantic NumWorks event to an edit or application intent.
    /// Function keys insert complete templates and put the cursor before `)`.
    pub fn handle_event(&mut self, value: event::Event) -> EditorAction {
        if self.picker_open {
            return self.handle_picker_event(value);
        }
        match value {
            event::LEFT => {
                if self.move_left() {
                    EditorAction::Changed
                } else {
                    EditorAction::None
                }
            }
            event::RIGHT => {
                if self.move_right() {
                    EditorAction::Changed
                } else {
                    EditorAction::None
                }
            }
            event::SHIFT_LEFT => {
                if self.move_to_start() {
                    EditorAction::Changed
                } else {
                    EditorAction::None
                }
            }
            event::SHIFT_RIGHT => {
                if self.move_to_end() {
                    EditorAction::Changed
                } else {
                    EditorAction::None
                }
            }
            event::OK => EditorAction::FocusTabs,
            event::BACK => EditorAction::Cancel,
            event::BACKSPACE => {
                if self.backspace() {
                    EditorAction::Changed
                } else {
                    EditorAction::None
                }
            }
            event::CLEAR => {
                if self.clear() {
                    EditorAction::Changed
                } else {
                    EditorAction::None
                }
            }
            event::EXE => EditorAction::Submit,
            event::XNT => self.insert_action(b"x"),
            event::TOOLBOX => {
                self.picker_open = true;
                self.picker_column = 0;
                self.picker_row = 0;
                EditorAction::Changed
            }
            event::SINE => self.insert_function_action(b"sin()"),
            event::COSINE => self.insert_function_action(b"cos()"),
            event::TANGENT => self.insert_function_action(b"tan()"),
            event::SQRT => self.insert_function_action(b"sqrt()"),
            event::SQUARE => self.insert_action(b"^2"),
            event::POWER => self.insert_action(b"^"),
            event::LEFT_PARENTHESIS => self.insert_action(b"("),
            event::RIGHT_PARENTHESIS => self.insert_action(b")"),
            event::MULTIPLICATION => self.insert_action(b"*"),
            event::DIVISION => self.insert_action(b"/"),
            event::PLUS => self.insert_action(b"+"),
            event::MINUS => self.insert_action(b"-"),
            event::DOT => self.insert_action(b"."),
            event::COMMA => self.insert_action(b","),
            event::EE => self.insert_action(b"e"),
            event::SPACE => self.insert_action(b" "),
            event::ZERO => self.insert_action(b"0"),
            event::ONE => self.insert_action(b"1"),
            event::TWO => self.insert_action(b"2"),
            event::THREE => self.insert_action(b"3"),
            event::FOUR => self.insert_action(b"4"),
            event::FIVE => self.insert_action(b"5"),
            event::SIX => self.insert_action(b"6"),
            event::SEVEN => self.insert_action(b"7"),
            event::EIGHT => self.insert_action(b"8"),
            event::NINE => self.insert_action(b"9"),
            _ => match event::lowercase_letter(value) {
                Some(letter) => self.insert_action(&[letter]),
                None => EditorAction::None,
            },
        }
    }

    fn handle_picker_event(&mut self, value: event::Event) -> EditorAction {
        match value {
            event::UP => {
                self.picker_row = if self.picker_row == 0 {
                    (FUNCTION_PICKER_ROWS - 1) as u8
                } else {
                    self.picker_row - 1
                };
                EditorAction::Changed
            }
            event::DOWN => {
                self.picker_row = (self.picker_row + 1) % FUNCTION_PICKER_ROWS as u8;
                EditorAction::Changed
            }
            event::LEFT | event::RIGHT => {
                self.picker_column ^= 1;
                EditorAction::Changed
            }
            event::EXE => {
                let template = self.selected_function_template();
                self.picker_open = false;
                self.insert_template_action(template)
            }
            event::BACK => {
                self.picker_open = false;
                EditorAction::Changed
            }
            event::OK => {
                self.picker_open = false;
                EditorAction::FocusTabs
            }
            _ => EditorAction::None,
        }
    }

    /// Compiles current text into `active` transactionally.
    /// `active` is written only after a successful complete parse.
    pub fn compile_into(&mut self, active: &mut CompiledExpression) -> bool {
        match CompiledExpression::compile(self.source()) {
            Ok(compiled) => {
                *active = compiled;
                self.error = None;
                true
            }
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }

    /// Clears an error without changing edited text or active bytecode.
    pub fn dismiss_error(&mut self) {
        self.error = None;
    }

    fn insert_action(&mut self, bytes: &[u8]) -> EditorAction {
        if self.insert_bytes(bytes) {
            EditorAction::Changed
        } else {
            EditorAction::None
        }
    }
    fn insert_function_action(&mut self, bytes: &[u8]) -> EditorAction {
        if self.insert_bytes(bytes) {
            let _ = self.move_left();
            EditorAction::Changed
        } else {
            EditorAction::None
        }
    }

    fn insert_template_action(&mut self, template: FunctionTemplate) -> EditorAction {
        let start = self.cursor;
        if !self.insert_bytes(template.source()) {
            return EditorAction::None;
        }
        self.cursor = if template.is_binary() {
            start + template.source().len() - 3
        } else {
            start + template.source().len() - 1
        };
        self.update_scroll();
        EditorAction::Changed
    }
    fn insert_bytes(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() > MAX_EXPRESSION_LENGTH - self.length {
            return false;
        }
        let old_cursor = self.cursor;
        let mut index = self.length;
        while index > old_cursor {
            self.buffer[index + bytes.len() - 1] = self.buffer[index - 1];
            index -= 1;
        }
        index = 0;
        while index < bytes.len() {
            self.buffer[old_cursor + index] = bytes[index];
            index += 1;
        }
        self.length += bytes.len();
        self.cursor += bytes.len();
        self.error = None;
        self.update_scroll();
        true
    }
    fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let removed = self.cursor - 1;
        let mut index = removed;
        while index + 1 < self.length {
            self.buffer[index] = self.buffer[index + 1];
            index += 1;
        }
        self.length -= 1;
        self.cursor -= 1;
        self.buffer[self.length] = 0;
        self.error = None;
        self.update_scroll();
        true
    }
    fn clear(&mut self) -> bool {
        if self.length == 0 {
            return false;
        }
        self.buffer = [0; MAX_EXPRESSION_LENGTH];
        self.length = 0;
        self.cursor = 0;
        self.scroll = 0;
        self.error = None;
        true
    }
    fn move_left(&mut self) -> bool {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.update_scroll();
            true
        } else {
            false
        }
    }
    fn move_right(&mut self) -> bool {
        if self.cursor < self.length {
            self.cursor += 1;
            self.update_scroll();
            true
        } else {
            false
        }
    }
    fn move_to_start(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        self.update_scroll();
        true
    }
    fn move_to_end(&mut self) -> bool {
        if self.cursor == self.length {
            return false;
        }
        self.cursor = self.length;
        self.update_scroll();
        true
    }
    fn update_scroll(&mut self) {
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor > self.scroll + VISIBLE_CHARACTERS {
            self.scroll = self.cursor - VISIBLE_CHARACTERS;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::function::SurfaceFunction;

    fn empty_editor() -> EquationEditor {
        let mut editor = EquationEditor::new();
        assert!(editor.clear());
        editor
    }

    #[test]
    fn inserts_at_beginning_middle_and_end() {
        let mut editor = empty_editor();
        assert!(editor.insert_bytes(b"ac"));
        let _ = editor.move_left();
        assert!(editor.insert_bytes(b"b"));
        assert_eq!(editor.source(), "abc");
        let _ = editor.move_left();
        let _ = editor.move_left();
        let _ = editor.move_left();
        assert!(editor.insert_bytes(b"0"));
        assert_eq!(editor.source(), "0abc");
        while editor.cursor() < editor.source().len() {
            let _ = editor.move_right();
        }
        assert!(editor.insert_bytes(b"z"));
        assert_eq!(editor.source(), "0abcz");
    }

    #[test]
    fn deletes_around_cursor() {
        let mut editor = empty_editor();
        assert!(editor.insert_bytes(b"abcd"));
        let _ = editor.move_left();
        let _ = editor.move_left();
        assert!(editor.backspace());
        assert_eq!(editor.source(), "acd");
        assert_eq!(editor.cursor(), 1);
        assert!(editor.backspace());
        assert_eq!(editor.source(), "cd");
        assert!(!editor.backspace());
    }

    #[test]
    fn cursor_stays_within_expression() {
        let mut editor = empty_editor();
        let _ = editor.move_left();
        assert_eq!(editor.cursor(), 0);
        assert!(editor.insert_bytes(b"xy"));
        let _ = editor.move_right();
        assert_eq!(editor.cursor(), 2);
    }

    #[test]
    fn capacity_is_enforced() {
        let mut editor = empty_editor();
        let full = [b'x'; MAX_EXPRESSION_LENGTH];
        assert!(editor.insert_bytes(&full));
        assert!(!editor.insert_bytes(b"y"));
        assert_eq!(editor.source().len(), MAX_EXPRESSION_LENGTH);
    }

    #[test]
    fn inserts_moves_across_and_deletes_spaces() {
        let mut editor = empty_editor();
        assert!(editor.insert_bytes(b"x"));
        assert_eq!(editor.handle_event(event::SPACE), EditorAction::Changed);
        assert!(editor.insert_bytes(b"y"));
        let _ = editor.move_left();
        let _ = editor.move_left();
        assert_eq!(editor.cursor(), 1);
        let _ = editor.move_right();
        assert_eq!(editor.cursor(), 2);
        assert!(editor.backspace());
        assert_eq!(editor.source(), "xy");
        assert_eq!(editor.cursor(), 1);
        assert!(editor.insert_bytes(b"  "));
        assert_eq!(editor.source(), "x  y");
    }

    #[test]
    fn deletion_at_beginning_and_end_is_safe() {
        let mut editor = empty_editor();
        assert!(!editor.backspace());
        assert!(editor.insert_bytes(b"x "));
        assert!(editor.backspace());
        assert_eq!(editor.source(), "x");
        assert!(editor.backspace());
        assert_eq!(editor.source(), "");
        assert!(!editor.backspace());
    }

    #[test]
    fn function_template_places_cursor_inside_parentheses() {
        let templates = [
            (event::SINE, "sin()", 4),
            (event::COSINE, "cos()", 4),
            (event::TANGENT, "tan()", 4),
            (event::SQRT, "sqrt()", 5),
        ];
        for (value, expected, cursor) in templates {
            let mut editor = empty_editor();
            assert_eq!(editor.handle_event(value), EditorAction::Changed);
            assert_eq!(editor.source(), expected);
            assert_eq!(editor.cursor(), cursor);
        }

        let mut sqrt_editor = empty_editor();
        assert_eq!(sqrt_editor.handle_event(event::SQRT), EditorAction::Changed);
        assert_eq!(sqrt_editor.handle_event(event::XNT), EditorAction::Changed);
        assert_eq!(
            sqrt_editor.handle_event(event::SQUARE),
            EditorAction::Changed
        );
        assert_eq!(sqrt_editor.source(), "sqrt(x^2)");
    }

    #[test]
    fn toolbox_picker_inserts_every_template_at_the_expected_cursor() {
        let mut column = 0_u8;
        while column < 2 {
            let mut row = 0_u8;
            while row < FUNCTION_PICKER_ROWS as u8 {
                let template = FunctionTemplate::from_position(column, row);
                let mut editor = empty_editor();
                assert_eq!(editor.handle_event(event::TOOLBOX), EditorAction::Changed);
                editor.picker_column = column;
                editor.picker_row = row;
                assert_eq!(editor.handle_event(event::EXE), EditorAction::Changed);
                assert_eq!(editor.source().as_bytes(), template.source());
                let expected_cursor = if template.is_binary() {
                    template.source().len() - 3
                } else {
                    template.source().len() - 1
                };
                assert_eq!(editor.cursor(), expected_cursor, "{:?}", template);
                assert!(!editor.function_picker_open());
                row += 1;
            }
            column += 1;
        }
    }

    #[test]
    fn toolbox_picker_wraps_switches_columns_and_cancels_without_editing() {
        let mut editor = empty_editor();
        assert!(editor.insert_bytes(b"x+"));
        assert_eq!(editor.handle_event(event::TOOLBOX), EditorAction::Changed);
        assert_eq!(editor.selected_function_template(), FunctionTemplate::Sin);
        assert_eq!(editor.handle_event(event::UP), EditorAction::Changed);
        assert_eq!(editor.selected_function_template(), FunctionTemplate::Round);
        assert_eq!(editor.handle_event(event::RIGHT), EditorAction::Changed);
        assert_eq!(editor.selected_function_template(), FunctionTemplate::Atan);
        assert_eq!(editor.handle_event(event::DOWN), EditorAction::Changed);
        assert_eq!(editor.selected_function_template(), FunctionTemplate::Exp);
        assert_eq!(editor.handle_event(event::BACK), EditorAction::Changed);
        assert!(!editor.function_picker_open());
        assert_eq!(editor.source(), "x+");
        assert_eq!(editor.handle_event(event::TOOLBOX), EditorAction::Changed);
        assert_eq!(editor.handle_event(event::OK), EditorAction::FocusTabs);
        assert!(!editor.function_picker_open());
        assert_eq!(editor.source(), "x+");
    }

    #[test]
    fn spaces_count_toward_the_fixed_capacity() {
        let mut editor = empty_editor();
        let full = [b' '; MAX_EXPRESSION_LENGTH];
        assert!(editor.insert_bytes(&full));
        assert!(!editor.insert_bytes(b" "));
        assert_eq!(editor.source().len(), MAX_EXPRESSION_LENGTH);
    }

    #[test]
    fn key_repeat_waits_then_repeats_only_editing_keys() {
        fn key(key: u8) -> keyboard::State {
            1_u64 << key
        }

        let mut repeat = EditorKeyRepeat::new();
        assert_eq!(repeat.update(key(keyboard::BACKSPACE), 100), None);
        assert_eq!(repeat.update(key(keyboard::BACKSPACE), 549), None);
        assert_eq!(
            repeat.update(key(keyboard::BACKSPACE), 550),
            Some(event::BACKSPACE)
        );
        assert_eq!(repeat.update(key(keyboard::BACKSPACE), 624), None);
        assert_eq!(
            repeat.update(key(keyboard::BACKSPACE), 625),
            Some(event::BACKSPACE)
        );
        assert_eq!(repeat.update(0, 626), None);
        assert_eq!(repeat.update(key(keyboard::LEFT), 700), None);
        assert_eq!(repeat.update(key(keyboard::LEFT), 1150), Some(event::LEFT));
        assert_eq!(repeat.update(key(keyboard::OK), 2000), None);
    }

    #[test]
    fn valid_replacement_updates_active_expression() {
        let mut active = CompiledExpression::compile("x").expect("initial expression");
        let mut editor = empty_editor();
        assert!(editor.insert_bytes(b"x^2+y^2"));
        assert!(editor.compile_into(&mut active));
        assert_eq!(active.evaluate(3.0, 4.0), 25.0);
    }

    #[test]
    fn whitespace_expression_compiles_identically() {
        let compact = CompiledExpression::compile("sin(x)*cos(y)").expect("compact expression");
        let spaced =
            CompiledExpression::compile("sin ( x ) * cos ( y )").expect("whitespace expression");
        assert!((compact.evaluate(0.75, -0.25) - spaced.evaluate(0.75, -0.25)).abs() < 0.0001);
    }

    #[test]
    fn failed_compile_preserves_active_expression() {
        let mut active = CompiledExpression::compile("x+y").expect("initial expression");
        let mut editor = empty_editor();
        assert!(editor.insert_bytes(b"sin("));
        assert!(!editor.compile_into(&mut active));
        assert_eq!(active.evaluate(2.0, 3.0), 5.0);
        assert!(editor.error().is_some());
        assert_eq!(editor.source(), "sin(");
    }

    #[test]
    fn invalid_function_arity_preserves_the_active_expression() {
        let mut active = CompiledExpression::compile("x+y").expect("initial expression");
        let mut editor = empty_editor();
        assert!(editor.insert_bytes(b"log(10)"));
        assert!(!editor.compile_into(&mut active));
        assert_eq!(active.evaluate(2.0, 3.0), 5.0);
        assert_eq!(editor.error(), Some(ParseError::InvalidArgumentCount));
    }
}

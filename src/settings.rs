//! Allocation-free Settings menu and transactional graph-domain editor.
//!
//! This module owns only Settings interaction state. It does not scan the
//! keyboard, poll EADK events, draw UI, mutate the camera, or own the active
//! domain. Those responsibilities remain in the application layer. Instead,
//! every operation returns a [`SettingsAction`] describing the smallest change
//! that the caller must apply. In particular, an invalid numeric draft can never
//! replace the last valid [`Domain`].
//!
//! Menu navigation is intended to use raw key-down edges. Numeric text entry uses
//! semantic EADK events, just like the Equation editor. The application may route
//! events synthesized by its existing held-key repeater here; Settings does not
//! own a second timer. Nothing here calls `eadk_event_get`, so it cannot block the
//! cooperative application loop.

use crate::eadk::event;
use crate::graph::{GraphOptions, Rgb888, SurfacePalette};
#[cfg(test)]
use crate::graph::{LightingPreset, RenderingMode};
use crate::surface::{Domain, DomainError};

/// Maximum source bytes in one domain-bound editor.
///
/// Twenty-four bytes comfortably hold every useful finite `f32` spelling while
/// keeping the editor's complete fixed storage negligible.
pub const NUMERIC_CAPACITY: usize = 24;
/// Number of fixed-width characters intended to fit in the Settings value field.
pub const NUMERIC_VISIBLE_CHARACTERS: usize = 20;
/// Number of selectable rows on the main Settings page.
pub const SETTINGS_ITEM_COUNT: usize = 8;
/// Number of editable bounds on the Domain page.
pub const DOMAIN_FIELD_COUNT: usize = 4;
/// Number of bounded choices on the Appearance page.
pub const APPEARANCE_ITEM_COUNT: usize = 4;
/// Three channels plus an explicit transactional Apply row.
pub const CUSTOM_COLOR_ITEM_COUNT: usize = 4;
/// Coarse channel adjustment used by Left/Right outside the numeric editor.
pub const CUSTOM_COLOR_STEP: u8 = 8;

/// Settings subview currently displayed inside the Settings tab.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingsPage {
    Main,
    Domain,
    Appearance,
    CustomColor,
}

/// Rows in the transactional Custom RGB editor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CustomColorItem {
    Red,
    Green,
    Blue,
    Apply,
}

impl CustomColorItem {
    pub fn index(self) -> usize {
        match self {
            CustomColorItem::Red => 0,
            CustomColorItem::Green => 1,
            CustomColorItem::Blue => 2,
            CustomColorItem::Apply => 3,
        }
    }

    pub fn from_index(index: u8) -> CustomColorItem {
        match index {
            0 => CustomColorItem::Red,
            1 => CustomColorItem::Green,
            2 => CustomColorItem::Blue,
            _ => CustomColorItem::Apply,
        }
    }
}

/// Rows in the Solid appearance submenu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AppearanceItem {
    Lighting,
    SurfaceColor,
    Resolution,
    AutoRotate,
}

impl AppearanceItem {
    pub fn index(self) -> usize {
        match self {
            AppearanceItem::Lighting => 0,
            AppearanceItem::SurfaceColor => 1,
            AppearanceItem::Resolution => 2,
            AppearanceItem::AutoRotate => 3,
        }
    }

    pub fn from_index(index: u8) -> AppearanceItem {
        match index {
            0 => AppearanceItem::Lighting,
            1 => AppearanceItem::SurfaceColor,
            2 => AppearanceItem::Resolution,
            _ => AppearanceItem::AutoRotate,
        }
    }
}

/// Rows in the compact main Settings menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingsItem {
    RenderingMode,
    GroundGrid,
    Axes,
    Ticks,
    Labels,
    Domain,
    ResetCamera,
    Performance,
}

impl SettingsItem {
    /// Stable zero-based row index used by the UI.
    pub fn index(self) -> usize {
        match self {
            SettingsItem::RenderingMode => 0,
            SettingsItem::GroundGrid => 1,
            SettingsItem::Axes => 2,
            SettingsItem::Ticks => 3,
            SettingsItem::Labels => 4,
            SettingsItem::Domain => 5,
            SettingsItem::ResetCamera => 6,
            SettingsItem::Performance => 7,
        }
    }

    /// Item for a bounded UI row index. Values beyond the final row clamp to
    /// `ResetCamera` so malformed state cannot cause an array-index panic.
    pub fn from_index(index: usize) -> SettingsItem {
        match index {
            0 => SettingsItem::RenderingMode,
            1 => SettingsItem::GroundGrid,
            2 => SettingsItem::Axes,
            3 => SettingsItem::Ticks,
            4 => SettingsItem::Labels,
            5 => SettingsItem::Domain,
            6 => SettingsItem::ResetCamera,
            _ => SettingsItem::Performance,
        }
    }
}

/// One editable component of the rectangular mathematical domain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DomainField {
    XMin,
    XMax,
    YMin,
    YMax,
}

impl DomainField {
    /// Stable zero-based row index used by the Domain page.
    pub fn index(self) -> usize {
        match self {
            DomainField::XMin => 0,
            DomainField::XMax => 1,
            DomainField::YMin => 2,
            DomainField::YMax => 3,
        }
    }

    /// Field for a bounded UI row index.
    pub fn from_index(index: usize) -> DomainField {
        match index {
            0 => DomainField::XMin,
            1 => DomainField::XMax,
            2 => DomainField::YMin,
            _ => DomainField::YMax,
        }
    }

    /// Reads this bound from a domain.
    pub fn value(self, domain: Domain) -> f32 {
        match self {
            DomainField::XMin => domain.x_min,
            DomainField::XMax => domain.x_max,
            DomainField::YMin => domain.y_min,
            DomainField::YMax => domain.y_max,
        }
    }

    fn replacing(self, domain: Domain, value: f32) -> Domain {
        match self {
            DomainField::XMin => Domain::new(value, domain.x_max, domain.y_min, domain.y_max),
            DomainField::XMax => Domain::new(domain.x_min, value, domain.y_min, domain.y_max),
            DomainField::YMin => Domain::new(domain.x_min, domain.x_max, value, domain.y_max),
            DomainField::YMax => Domain::new(domain.x_min, domain.x_max, domain.y_min, value),
        }
    }
}

/// User-facing failure retained while the numeric field remains active.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NumericError {
    InvalidNumber,
    TooLong,
    Domain(DomainError),
    ColorOutOfRange,
}

/// Result for the application layer to translate into dirty flags/state changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingsAction {
    /// No state or pixels changed.
    None,
    /// Settings content changed, but graph data did not.
    Redraw,
    /// `GraphOptions` changed; do not resample the mathematical surface.
    GraphChanged,
    /// Sampling density changed; the current surface must be rebuilt before rendering.
    ResolutionChanged,
    /// Toggles the transient application camera animation state.
    AutoRotateChanged,
    /// A fully validated domain should transactionally replace the active one.
    DomainChanged(Domain),
    /// The application should restore its established default camera.
    ResetCamera,
    /// Back was pressed at the main Settings page; return to Graph content.
    LeaveSettings,
}

/// Small NUL-terminated number produced without formatting machinery or a heap.
#[derive(Clone, Copy)]
pub struct NumberText {
    bytes: [u8; NUMERIC_CAPACITY + 1],
    length: usize,
}

impl NumberText {
    /// Formats a finite graph bound with up to six fractional digits.
    ///
    /// Six decimals are adequate for the configured domain limits and calculator
    /// display. A non-finite input is represented as `?`, although active domains
    /// should already have passed `Domain::validate`.
    pub fn new(value: f32) -> NumberText {
        let mut text = NumberText {
            bytes: [0; NUMERIC_CAPACITY + 1],
            length: 0,
        };
        text.write(value);
        text
    }

    /// Bytes excluding the C terminator.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }

    /// NUL-terminated bytes suitable for `eadk_display_draw_string`.
    pub fn as_c_string(&self) -> &[u8] {
        &self.bytes[..=self.length]
    }

    fn write(&mut self, value: f32) {
        self.length = 0;
        self.bytes = [0; NUMERIC_CAPACITY + 1];
        if !value.is_finite() {
            self.push(b'?');
            return;
        }

        let negative = value < 0.0;
        let absolute = if negative { -value } else { value };
        let mut integer = absolute as u32;
        let mut fraction = ((absolute - integer as f32) * 1_000_000.0 + 0.5) as u32;
        if fraction >= 1_000_000 {
            integer = integer.saturating_add(1);
            fraction -= 1_000_000;
        }

        if negative {
            self.push(b'-');
        }
        self.write_unsigned(integer);
        if fraction != 0 {
            self.push(b'.');
            let fraction_start = self.length;
            let mut divisor = 100_000_u32;
            while divisor > 0 {
                self.push(b'0' + ((fraction / divisor) % 10) as u8);
                divisor /= 10;
            }
            while self.length > fraction_start && self.bytes[self.length - 1] == b'0' {
                self.length -= 1;
                self.bytes[self.length] = 0;
            }
        }
    }

    fn write_unsigned(&mut self, value: u32) {
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
        while count > 0 {
            count -= 1;
            self.push(reverse[count]);
        }
    }

    fn push(&mut self, byte: u8) {
        if self.length < NUMERIC_CAPACITY {
            self.bytes[self.length] = byte;
            self.length += 1;
            self.bytes[self.length] = 0;
        }
    }
}

/// Fixed-storage state for every Settings subpage and one shared numeric field.
///
/// Custom RGB drafts remain separate from `GraphOptions` until Apply, so Back
/// cannot partially change the active Solid appearance.
pub struct SettingsState {
    page: SettingsPage,
    menu_index: usize,
    domain_index: usize,
    appearance_index: u8,
    custom_color_index: u8,
    custom_color_draft: Rgb888,
    editing: bool,
    numeric: NumericEditor,
}

impl SettingsState {
    /// Starts at the Rendering mode row on the main page.
    pub fn new() -> SettingsState {
        SettingsState {
            page: SettingsPage::Main,
            menu_index: 0,
            domain_index: 0,
            appearance_index: 0,
            custom_color_index: 0,
            custom_color_draft: Rgb888::DEFAULT_CUSTOM,
            editing: false,
            numeric: NumericEditor::new(),
        }
    }

    /// Currently visible Settings page.
    pub fn page(&self) -> SettingsPage {
        self.page
    }

    /// Selected main-menu row. It remains available while the Domain page is open.
    pub fn selected_item(&self) -> SettingsItem {
        SettingsItem::from_index(self.menu_index)
    }

    /// Selected bound on the Domain page.
    pub fn selected_domain_field(&self) -> DomainField {
        DomainField::from_index(self.domain_index)
    }

    /// Selected row on the bounded Solid appearance page.
    pub fn selected_appearance_item(&self) -> AppearanceItem {
        AppearanceItem::from_index(self.appearance_index)
    }

    /// Selected R/G/B/Apply row in the transactional Custom Color page.
    pub fn selected_custom_color_item(&self) -> CustomColorItem {
        CustomColorItem::from_index(self.custom_color_index)
    }

    /// Temporary RGB value shown by the Custom Color page and preview.
    pub fn custom_color_draft(&self) -> Rgb888 {
        self.custom_color_draft
    }

    /// Whether semantic events currently belong to the numeric field.
    pub fn is_editing(&self) -> bool {
        self.editing
    }

    /// Error to draw below the active numeric field.
    pub fn error(&self) -> Option<NumericError> {
        self.numeric.error
    }

    /// Complete ASCII numeric draft used by host-side transactional tests.
    #[cfg(test)]
    pub fn edit_source(&self) -> &str {
        self.numeric.source()
    }

    /// Current byte/character insertion position.
    pub fn edit_cursor(&self) -> usize {
        self.numeric.cursor
    }

    /// First visible byte in the horizontally scrolled numeric field.
    pub fn edit_scroll(&self) -> usize {
        self.numeric.scroll
    }

    /// Visible portion of the numeric draft.
    pub fn edit_visible_bytes(&self) -> &[u8] {
        self.numeric.visible_bytes()
    }

    /// Moves one row upward without wrapping. Intended for a raw down edge.
    pub fn select_previous(&mut self) -> SettingsAction {
        if self.editing {
            return SettingsAction::None;
        }
        match self.page {
            SettingsPage::Main => decrement(&mut self.menu_index),
            SettingsPage::Domain => decrement(&mut self.domain_index),
            SettingsPage::Appearance => decrement_u8(&mut self.appearance_index),
            SettingsPage::CustomColor => decrement_u8(&mut self.custom_color_index),
        }
    }

    /// Moves one row downward without wrapping. Intended for a raw down edge.
    pub fn select_next(&mut self) -> SettingsAction {
        if self.editing {
            return SettingsAction::None;
        }
        match self.page {
            SettingsPage::Main => increment(&mut self.menu_index, SETTINGS_ITEM_COUNT),
            SettingsPage::Domain => increment(&mut self.domain_index, DOMAIN_FIELD_COUNT),
            SettingsPage::Appearance => {
                increment_u8(&mut self.appearance_index, APPEARANCE_ITEM_COUNT as u8)
            }
            SettingsPage::CustomColor => {
                increment_u8(&mut self.custom_color_index, CUSTOM_COLOR_ITEM_COUNT as u8)
            }
        }
    }

    /// Applies the left-arrow meaning for the selected main-menu value.
    pub fn adjust_left(&mut self, options: &mut GraphOptions) -> SettingsAction {
        self.adjust(options, false)
    }

    /// Applies the right-arrow meaning for the selected main-menu value.
    pub fn adjust_right(&mut self, options: &mut GraphOptions) -> SettingsAction {
        self.adjust(options, true)
    }

    /// Handles EXE/activation independently of physical raw-key constants.
    ///
    /// Main value rows cycle/toggle, Domain opens its submenu, Reset emits an
    /// application-owned action, and a Domain row begins or submits numeric edit.
    pub fn activate(
        &mut self,
        options: &mut GraphOptions,
        active_domain: Domain,
    ) -> SettingsAction {
        if self.page == SettingsPage::Domain {
            if self.editing {
                return self.submit_domain(active_domain);
            }
            self.begin_domain_edit(active_domain);
            return SettingsAction::Redraw;
        }
        if self.page == SettingsPage::CustomColor {
            if self.editing {
                return self.submit_custom_channel();
            }
            if self.selected_custom_color_item() == CustomColorItem::Apply {
                options.custom_rgb = self.custom_color_draft;
                options.surface_palette = SurfacePalette::Custom;
                self.page = SettingsPage::Appearance;
                self.numeric.reset();
                return SettingsAction::GraphChanged;
            }
            self.begin_custom_channel_edit();
            return SettingsAction::Redraw;
        }
        if self.page == SettingsPage::Appearance {
            if self.selected_appearance_item() == AppearanceItem::SurfaceColor
                && options.surface_palette == SurfacePalette::Custom
            {
                self.page = SettingsPage::CustomColor;
                self.custom_color_index = 0;
                self.custom_color_draft = options.custom_rgb;
                self.numeric.reset();
                return SettingsAction::Redraw;
            }
            return self.adjust_appearance(options, true);
        }

        match self.selected_item() {
            SettingsItem::RenderingMode => {
                self.page = SettingsPage::Appearance;
                self.appearance_index = 0;
                SettingsAction::Redraw
            }
            SettingsItem::GroundGrid => {
                options.show_grid = !options.show_grid;
                SettingsAction::GraphChanged
            }
            SettingsItem::Axes => {
                options.show_axes = !options.show_axes;
                SettingsAction::GraphChanged
            }
            SettingsItem::Ticks => {
                options.show_ticks = !options.show_ticks;
                SettingsAction::GraphChanged
            }
            SettingsItem::Labels => {
                options.show_labels = !options.show_labels;
                SettingsAction::GraphChanged
            }
            SettingsItem::Performance => {
                options.show_performance = !options.show_performance;
                SettingsAction::Redraw
            }
            SettingsItem::Domain => {
                self.page = SettingsPage::Domain;
                self.editing = false;
                self.numeric.reset();
                SettingsAction::Redraw
            }
            SettingsItem::ResetCamera => SettingsAction::ResetCamera,
        }
    }

    /// Handles Back at every nested Settings level.
    pub fn back(&mut self) -> SettingsAction {
        if self.editing {
            self.editing = false;
            self.numeric.reset();
            return SettingsAction::Redraw;
        }
        if self.page == SettingsPage::CustomColor {
            self.page = SettingsPage::Appearance;
            self.numeric.reset();
            return SettingsAction::Redraw;
        }
        if self.page == SettingsPage::Domain || self.page == SettingsPage::Appearance {
            self.page = SettingsPage::Main;
            self.numeric.reset();
            return SettingsAction::Redraw;
        }
        SettingsAction::LeaveSettings
    }

    /// Applies one semantic calculator event while a numeric field is active.
    ///
    /// OK is deliberately ignored here because tab focus remains application-
    /// owned. EXE submits, Back cancels, and only useful numeric characters are
    /// accepted; Alpha letters and function shortcuts cannot enter a bound.
    pub fn handle_editor_event(
        &mut self,
        value: event::Event,
        active_domain: Domain,
    ) -> SettingsAction {
        if !self.editing {
            return SettingsAction::None;
        }
        match value {
            event::LEFT => self.numeric_move_left(),
            event::RIGHT => self.numeric_move_right(),
            event::SHIFT_LEFT => self.numeric_move_start(),
            event::SHIFT_RIGHT => self.numeric_move_end(),
            event::BACKSPACE => self.numeric_backspace(),
            event::CLEAR => self.numeric_clear(),
            event::BACK => self.back(),
            event::EXE => {
                if self.page == SettingsPage::CustomColor {
                    self.submit_custom_channel()
                } else {
                    self.submit_domain(active_domain)
                }
            }
            event::ZERO => self.numeric_insert(b"0"),
            event::ONE => self.numeric_insert(b"1"),
            event::TWO => self.numeric_insert(b"2"),
            event::THREE => self.numeric_insert(b"3"),
            event::FOUR => self.numeric_insert(b"4"),
            event::FIVE => self.numeric_insert(b"5"),
            event::SIX => self.numeric_insert(b"6"),
            event::SEVEN => self.numeric_insert(b"7"),
            event::EIGHT => self.numeric_insert(b"8"),
            event::NINE => self.numeric_insert(b"9"),
            event::DOT if self.page != SettingsPage::CustomColor => self.numeric_insert(b"."),
            event::MINUS if self.page != SettingsPage::CustomColor => self.numeric_insert(b"-"),
            event::PLUS if self.page != SettingsPage::CustomColor => self.numeric_insert(b"+"),
            event::EE if self.page != SettingsPage::CustomColor => self.numeric_insert(b"e"),
            _ => SettingsAction::None,
        }
    }

    fn adjust(&mut self, options: &mut GraphOptions, right: bool) -> SettingsAction {
        if self.editing {
            return SettingsAction::None;
        }
        if self.page == SettingsPage::Appearance {
            return self.adjust_appearance(options, right);
        }
        if self.page == SettingsPage::CustomColor {
            return self.adjust_custom_color(right);
        }
        if self.page != SettingsPage::Main {
            return SettingsAction::None;
        }
        let changed = match self.selected_item() {
            SettingsItem::RenderingMode => {
                options.rendering_mode = if right {
                    options.rendering_mode.next()
                } else {
                    options.rendering_mode.previous()
                };
                true
            }
            SettingsItem::GroundGrid => set_bool(&mut options.show_grid, right),
            SettingsItem::Axes => set_bool(&mut options.show_axes, right),
            SettingsItem::Ticks => set_bool(&mut options.show_ticks, right),
            SettingsItem::Labels => set_bool(&mut options.show_labels, right),
            SettingsItem::Performance => set_bool(&mut options.show_performance, right),
            SettingsItem::Domain | SettingsItem::ResetCamera => false,
        };
        if changed {
            if self.selected_item() == SettingsItem::Performance {
                // The readout is UI-only; toggling it must not invalidate graph pixels.
                SettingsAction::Redraw
            } else {
                SettingsAction::GraphChanged
            }
        } else {
            SettingsAction::None
        }
    }

    fn adjust_appearance(&mut self, options: &mut GraphOptions, right: bool) -> SettingsAction {
        match self.selected_appearance_item() {
            AppearanceItem::Lighting => {
                options.lighting = if right {
                    options.lighting.next()
                } else {
                    options.lighting.previous()
                };
            }
            AppearanceItem::SurfaceColor => {
                options.surface_palette = if right {
                    options.surface_palette.next()
                } else {
                    options.surface_palette.previous()
                };
            }
            AppearanceItem::Resolution => {
                options.resolution = if right {
                    options.resolution.next()
                } else {
                    options.resolution.previous()
                };
                return SettingsAction::ResolutionChanged;
            }
            AppearanceItem::AutoRotate => return SettingsAction::AutoRotateChanged,
        }
        SettingsAction::GraphChanged
    }

    fn begin_domain_edit(&mut self, active_domain: Domain) {
        let value = self.selected_domain_field().value(active_domain);
        self.numeric.load(value);
        self.editing = true;
    }

    fn begin_custom_channel_edit(&mut self) {
        let value = self.custom_channel_value();
        self.numeric.load(value as f32);
        self.editing = true;
    }

    fn submit_custom_channel(&mut self) -> SettingsAction {
        if !self.numeric.modified {
            self.editing = false;
            self.numeric.reset();
            return SettingsAction::Redraw;
        }
        let value = match parse_color_channel(self.numeric.source().as_bytes()) {
            Some(value) => value,
            None => {
                self.numeric.error = Some(NumericError::ColorOutOfRange);
                return SettingsAction::Redraw;
            }
        };
        self.set_custom_channel_value(value);
        self.editing = false;
        self.numeric.reset();
        SettingsAction::Redraw
    }

    fn adjust_custom_color(&mut self, right: bool) -> SettingsAction {
        if self.selected_custom_color_item() == CustomColorItem::Apply {
            return SettingsAction::None;
        }
        let current = self.custom_channel_value();
        let adjusted = if right {
            current.saturating_add(CUSTOM_COLOR_STEP)
        } else {
            current.saturating_sub(CUSTOM_COLOR_STEP)
        };
        if adjusted == current {
            SettingsAction::None
        } else {
            self.set_custom_channel_value(adjusted);
            SettingsAction::Redraw
        }
    }

    fn custom_channel_value(&self) -> u8 {
        match self.selected_custom_color_item() {
            CustomColorItem::Red => self.custom_color_draft.red,
            CustomColorItem::Green => self.custom_color_draft.green,
            CustomColorItem::Blue => self.custom_color_draft.blue,
            CustomColorItem::Apply => 0,
        }
    }

    fn set_custom_channel_value(&mut self, value: u8) {
        match self.selected_custom_color_item() {
            CustomColorItem::Red => self.custom_color_draft.red = value,
            CustomColorItem::Green => self.custom_color_draft.green = value,
            CustomColorItem::Blue => self.custom_color_draft.blue = value,
            CustomColorItem::Apply => {}
        }
    }

    fn submit_domain(&mut self, active_domain: Domain) -> SettingsAction {
        // Merely opening and accepting the field must not round the active f32 to
        // the six decimals used for its compact display representation.
        if !self.numeric.modified {
            self.editing = false;
            self.numeric.reset();
            return SettingsAction::Redraw;
        }
        let value = match parse_number(self.numeric.source().as_bytes()) {
            Some(value) => value,
            None => {
                self.numeric.error = Some(NumericError::InvalidNumber);
                return SettingsAction::Redraw;
            }
        };
        let candidate = self.selected_domain_field().replacing(active_domain, value);
        if let Err(error) = candidate.validate() {
            self.numeric.error = Some(NumericError::Domain(error));
            return SettingsAction::Redraw;
        }
        self.editing = false;
        self.numeric.reset();
        SettingsAction::DomainChanged(candidate)
    }

    fn numeric_insert(&mut self, bytes: &[u8]) -> SettingsAction {
        if self.numeric.insert(bytes) {
            SettingsAction::Redraw
        } else {
            self.numeric.error = Some(NumericError::TooLong);
            SettingsAction::Redraw
        }
    }

    fn numeric_backspace(&mut self) -> SettingsAction {
        if self.numeric.backspace() {
            SettingsAction::Redraw
        } else {
            SettingsAction::None
        }
    }

    fn numeric_clear(&mut self) -> SettingsAction {
        if self.numeric.clear() {
            SettingsAction::Redraw
        } else {
            SettingsAction::None
        }
    }

    fn numeric_move_left(&mut self) -> SettingsAction {
        if self.numeric.move_left() {
            SettingsAction::Redraw
        } else {
            SettingsAction::None
        }
    }

    fn numeric_move_right(&mut self) -> SettingsAction {
        if self.numeric.move_right() {
            SettingsAction::Redraw
        } else {
            SettingsAction::None
        }
    }

    fn numeric_move_start(&mut self) -> SettingsAction {
        if self.numeric.move_to_start() {
            SettingsAction::Redraw
        } else {
            SettingsAction::None
        }
    }

    fn numeric_move_end(&mut self) -> SettingsAction {
        if self.numeric.move_to_end() {
            SettingsAction::Redraw
        } else {
            SettingsAction::None
        }
    }
}

struct NumericEditor {
    buffer: [u8; NUMERIC_CAPACITY + 1],
    length: usize,
    cursor: usize,
    scroll: usize,
    error: Option<NumericError>,
    modified: bool,
}

impl NumericEditor {
    fn new() -> NumericEditor {
        NumericEditor {
            buffer: [0; NUMERIC_CAPACITY + 1],
            length: 0,
            cursor: 0,
            scroll: 0,
            error: None,
            modified: false,
        }
    }

    fn reset(&mut self) {
        self.buffer = [0; NUMERIC_CAPACITY + 1];
        self.length = 0;
        self.cursor = 0;
        self.scroll = 0;
        self.error = None;
        self.modified = false;
    }

    fn load(&mut self, value: f32) {
        self.reset();
        let text = NumberText::new(value);
        let _ = self.insert(text.as_bytes());
        self.error = None;
        self.modified = false;
    }

    fn source(&self) -> &str {
        match core::str::from_utf8(&self.buffer[..self.length]) {
            Ok(source) => source,
            Err(_) => "",
        }
    }

    fn visible_bytes(&self) -> &[u8] {
        let end = core::cmp::min(self.length, self.scroll + NUMERIC_VISIBLE_CHARACTERS);
        &self.buffer[self.scroll..end]
    }

    fn insert(&mut self, bytes: &[u8]) -> bool {
        if bytes.is_empty() || bytes.len() > NUMERIC_CAPACITY - self.length {
            return false;
        }
        let mut index = self.length;
        while index > self.cursor {
            self.buffer[index + bytes.len() - 1] = self.buffer[index - 1];
            index -= 1;
        }
        index = 0;
        while index < bytes.len() {
            self.buffer[self.cursor + index] = bytes[index];
            index += 1;
        }
        self.length += bytes.len();
        self.cursor += bytes.len();
        self.buffer[self.length] = 0;
        self.error = None;
        self.modified = true;
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
        self.modified = true;
        self.update_scroll();
        true
    }

    fn clear(&mut self) -> bool {
        if self.length == 0 {
            return false;
        }
        self.buffer = [0; NUMERIC_CAPACITY + 1];
        self.length = 0;
        self.cursor = 0;
        self.scroll = 0;
        self.error = None;
        self.modified = true;
        true
    }

    fn move_left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.update_scroll();
        true
    }

    fn move_right(&mut self) -> bool {
        if self.cursor >= self.length {
            return false;
        }
        self.cursor += 1;
        self.update_scroll();
        true
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
        } else if self.cursor > self.scroll + NUMERIC_VISIBLE_CHARACTERS {
            self.scroll = self.cursor - NUMERIC_VISIBLE_CHARACTERS;
        }
    }
}

fn set_bool(value: &mut bool, new_value: bool) -> bool {
    if *value == new_value {
        false
    } else {
        *value = new_value;
        true
    }
}

fn decrement(index: &mut usize) -> SettingsAction {
    if *index == 0 {
        SettingsAction::None
    } else {
        *index -= 1;
        SettingsAction::Redraw
    }
}

fn increment(index: &mut usize, count: usize) -> SettingsAction {
    if *index + 1 >= count {
        SettingsAction::None
    } else {
        *index += 1;
        SettingsAction::Redraw
    }
}

fn decrement_u8(index: &mut u8) -> SettingsAction {
    if *index == 0 {
        SettingsAction::None
    } else {
        *index -= 1;
        SettingsAction::Redraw
    }
}

fn increment_u8(index: &mut u8, count: u8) -> SettingsAction {
    if index.saturating_add(1) >= count {
        SettingsAction::None
    } else {
        *index += 1;
        SettingsAction::Redraw
    }
}

/// Parses one Custom RGB channel without floating-point conversion.
fn parse_color_channel(source: &[u8]) -> Option<u8> {
    if source.is_empty() {
        return None;
    }
    let mut value = 0_u16;
    let mut index = 0;
    while index < source.len() {
        let byte = source[index];
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value
            .saturating_mul(10)
            .saturating_add((byte - b'0') as u16);
        if value > u8::MAX as u16 {
            return None;
        }
        index += 1;
    }
    Some(value as u8)
}

/// Parses the deliberately small numeric grammar accepted by a bound field:
/// optional sign, decimal mantissa, and optional signed base-ten exponent.
fn parse_number(source: &[u8]) -> Option<f32> {
    if source.is_empty() {
        return None;
    }
    let mut position = 0;
    let mut negative = false;
    if source[position] == b'+' || source[position] == b'-' {
        negative = source[position] == b'-';
        position += 1;
    }

    let mut value = 0.0_f32;
    let mut digits = 0;
    while position < source.len() && source[position].is_ascii_digit() {
        value = value * 10.0 + (source[position] - b'0') as f32;
        position += 1;
        digits += 1;
    }
    if position < source.len() && source[position] == b'.' {
        position += 1;
        let mut place = 0.1_f32;
        while position < source.len() && source[position].is_ascii_digit() {
            value += (source[position] - b'0') as f32 * place;
            place *= 0.1;
            position += 1;
            digits += 1;
        }
    }
    if digits == 0 || !value.is_finite() {
        return None;
    }

    if position < source.len() && (source[position] == b'e' || source[position] == b'E') {
        position += 1;
        let mut exponent_negative = false;
        if position < source.len() && (source[position] == b'+' || source[position] == b'-') {
            exponent_negative = source[position] == b'-';
            position += 1;
        }
        let mut exponent = 0_u16;
        let mut exponent_digits = 0;
        while position < source.len() && source[position].is_ascii_digit() {
            exponent = exponent
                .saturating_mul(10)
                .saturating_add((source[position] - b'0') as u16);
            position += 1;
            exponent_digits += 1;
        }
        if exponent_digits == 0 || exponent > 38 {
            return None;
        }
        let mut factor = 1.0_f32;
        let mut power = 0;
        while power < exponent {
            factor *= 10.0;
            power += 1;
        }
        value = if exponent_negative {
            value / factor
        } else {
            value * factor
        };
    }

    if position != source.len() || !value.is_finite() {
        return None;
    }
    Some(if negative { -value } else { value })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn move_to(state: &mut SettingsState, item: SettingsItem) {
        while state.selected_item().index() < item.index() {
            assert_eq!(state.select_next(), SettingsAction::Redraw);
        }
    }

    fn enter_domain(state: &mut SettingsState, options: &mut GraphOptions) {
        move_to(state, SettingsItem::Domain);
        assert_eq!(
            state.activate(options, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        assert_eq!(state.page(), SettingsPage::Domain);
    }

    fn enter_appearance(state: &mut SettingsState, options: &mut GraphOptions) {
        assert_eq!(
            state.activate(options, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        assert_eq!(state.page(), SettingsPage::Appearance);
    }

    fn enter_custom_color(state: &mut SettingsState, options: &mut GraphOptions) {
        enter_appearance(state, options);
        assert_eq!(state.select_next(), SettingsAction::Redraw);
        options.surface_palette = SurfacePalette::Custom;
        assert_eq!(
            state.activate(options, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        assert_eq!(state.page(), SettingsPage::CustomColor);
    }

    fn replace_edit_text(state: &mut SettingsState, domain: Domain, events: &[event::Event]) {
        let mut options = GraphOptions::DEFAULT;
        assert_eq!(state.activate(&mut options, domain), SettingsAction::Redraw);
        assert!(state.is_editing());
        assert_eq!(
            state.handle_editor_event(event::CLEAR, domain),
            SettingsAction::Redraw
        );
        for value in events {
            assert_eq!(
                state.handle_editor_event(*value, domain),
                SettingsAction::Redraw
            );
        }
    }

    #[test]
    fn mode_and_boolean_rows_update_graph_options() {
        let mut state = SettingsState::new();
        let mut options = GraphOptions::DEFAULT;
        assert_eq!(
            state.adjust_right(&mut options),
            SettingsAction::GraphChanged
        );
        assert_eq!(options.rendering_mode, RenderingMode::Solid);
        assert_eq!(
            state.adjust_left(&mut options),
            SettingsAction::GraphChanged
        );
        assert_eq!(options.rendering_mode, RenderingMode::Wireframe);

        assert_eq!(state.select_next(), SettingsAction::Redraw);
        assert_eq!(state.selected_item(), SettingsItem::GroundGrid);
        assert_eq!(
            state.adjust_left(&mut options),
            SettingsAction::GraphChanged
        );
        assert!(!options.show_grid);
        assert_eq!(state.adjust_left(&mut options), SettingsAction::None);
        assert_eq!(
            state.activate(&mut options, Domain::DEFAULT),
            SettingsAction::GraphChanged
        );
        assert!(options.show_grid);

        assert_eq!(state.select_next(), SettingsAction::Redraw);
        assert_eq!(
            state.activate(&mut options, Domain::DEFAULT),
            SettingsAction::GraphChanged
        );
        assert!(!options.show_axes);
        assert_eq!(state.select_next(), SettingsAction::Redraw);
        assert_eq!(
            state.activate(&mut options, Domain::DEFAULT),
            SettingsAction::GraphChanged
        );
        assert!(!options.show_ticks);
        assert_eq!(state.select_next(), SettingsAction::Redraw);
        assert_eq!(
            state.activate(&mut options, Domain::DEFAULT),
            SettingsAction::GraphChanged
        );
        assert!(!options.show_labels);
    }

    #[test]
    fn appearance_page_cycles_lighting_and_color_without_resampling() {
        let mut state = SettingsState::new();
        let mut options = GraphOptions::DEFAULT;
        assert_eq!(
            state.activate(&mut options, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        assert_eq!(state.page(), SettingsPage::Appearance);
        assert_eq!(state.selected_appearance_item(), AppearanceItem::Lighting);

        assert_eq!(
            state.adjust_right(&mut options),
            SettingsAction::GraphChanged
        );
        assert_eq!(options.lighting, LightingPreset::Soft);
        assert_eq!(
            state.activate(&mut options, Domain::DEFAULT),
            SettingsAction::GraphChanged
        );
        assert_eq!(options.lighting, LightingPreset::Strong);
        assert_eq!(state.select_next(), SettingsAction::Redraw);
        assert_eq!(
            state.selected_appearance_item(),
            AppearanceItem::SurfaceColor
        );
        assert_eq!(
            state.adjust_left(&mut options),
            SettingsAction::GraphChanged
        );
        assert_eq!(options.surface_palette, SurfacePalette::Custom);
        assert_eq!(state.select_next(), SettingsAction::Redraw);
        assert_eq!(state.selected_appearance_item(), AppearanceItem::Resolution);
        assert_eq!(
            state.adjust_right(&mut options),
            SettingsAction::ResolutionChanged
        );
        assert_eq!(state.back(), SettingsAction::Redraw);
        assert_eq!(state.page(), SettingsPage::Main);
    }

    #[test]
    fn appearance_defaults_and_cycles_are_bounded() {
        let mut options = GraphOptions::DEFAULT;
        assert_eq!(options.lighting, LightingPreset::Standard);
        assert_eq!(options.surface_palette, SurfacePalette::Blue);
        options.lighting = options.lighting.previous();
        options.surface_palette = options.surface_palette.previous();
        assert_eq!(options.lighting, LightingPreset::Strong);
        assert_eq!(options.surface_palette, SurfacePalette::Custom);
        assert_eq!(options.lighting.next(), LightingPreset::Standard);
        assert_eq!(options.surface_palette.next(), SurfacePalette::Blue);
    }

    #[test]
    fn appearance_resolution_cycles_and_reports_surface_resample_action() {
        let mut state = SettingsState::new();
        let mut options = GraphOptions::DEFAULT;
        assert_eq!(
            state.activate(&mut options, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        assert_eq!(state.select_next(), SettingsAction::Redraw);
        assert_eq!(state.select_next(), SettingsAction::Redraw);
        assert_eq!(state.selected_appearance_item(), AppearanceItem::Resolution);
        assert_eq!(
            state.adjust_left(&mut options),
            SettingsAction::ResolutionChanged
        );
        assert_eq!(options.resolution, crate::surface::ResolutionPreset::Low);
        assert_eq!(
            state.adjust_right(&mut options),
            SettingsAction::ResolutionChanged
        );
        assert_eq!(
            options.resolution,
            crate::surface::ResolutionPreset::Standard
        );
        assert_eq!(
            state.adjust_right(&mut options),
            SettingsAction::ResolutionChanged
        );
        assert_eq!(options.resolution, crate::surface::ResolutionPreset::High);
        assert_eq!(
            state.adjust_right(&mut options),
            SettingsAction::ResolutionChanged
        );
        assert_eq!(options.resolution, crate::surface::ResolutionPreset::Ultra);
        assert_eq!(
            state.adjust_right(&mut options),
            SettingsAction::ResolutionChanged
        );
        assert_eq!(options.resolution, crate::surface::ResolutionPreset::Low);
        assert_eq!(state.select_next(), SettingsAction::Redraw);
        assert_eq!(state.selected_appearance_item(), AppearanceItem::AutoRotate);
        assert_eq!(
            state.adjust_right(&mut options),
            SettingsAction::AutoRotateChanged
        );
    }

    #[test]
    fn built_in_and_custom_palette_cycle_is_complete_and_bounded() {
        let mut palette = SurfacePalette::Blue;
        let expected = [
            SurfacePalette::Green,
            SurfacePalette::Orange,
            SurfacePalette::Purple,
            SurfacePalette::Gray,
            SurfacePalette::Red,
            SurfacePalette::Cyan,
            SurfacePalette::Yellow,
            SurfacePalette::White,
            SurfacePalette::Custom,
            SurfacePalette::Blue,
        ];
        for next in expected {
            palette = palette.next();
            assert_eq!(palette, next);
        }
        assert_eq!(SurfacePalette::Blue.previous(), SurfacePalette::Custom);
        assert_eq!(SurfacePalette::Custom.previous(), SurfacePalette::White);
    }

    #[test]
    fn custom_color_adjustment_is_temporary_and_saturates() {
        let mut state = SettingsState::new();
        let mut options = GraphOptions::DEFAULT;
        let active = options.custom_rgb;
        enter_custom_color(&mut state, &mut options);

        assert_eq!(state.custom_color_draft(), active);
        assert_eq!(state.adjust_right(&mut options), SettingsAction::Redraw);
        assert_eq!(
            state.custom_color_draft().red,
            active.red + CUSTOM_COLOR_STEP
        );
        assert_eq!(options.custom_rgb, active);

        let mut count = 0;
        while state.adjust_right(&mut options) == SettingsAction::Redraw {
            count += 1;
            assert!(count < 40);
        }
        assert_eq!(state.custom_color_draft().red, 255);
        while state.adjust_left(&mut options) == SettingsAction::Redraw {}
        assert_eq!(state.custom_color_draft().red, 0);
        assert_eq!(options.custom_rgb, active);
    }

    #[test]
    fn custom_channel_numeric_edit_validates_and_apply_commits_atomically() {
        let mut state = SettingsState::new();
        let mut options = GraphOptions::DEFAULT;
        let original = options.custom_rgb;
        enter_custom_color(&mut state, &mut options);

        assert_eq!(
            state.activate(&mut options, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        assert!(state.is_editing());
        assert_eq!(
            state.handle_editor_event(event::CLEAR, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        for value in [event::TWO, event::FIVE, event::SIX] {
            assert_eq!(
                state.handle_editor_event(value, Domain::DEFAULT),
                SettingsAction::Redraw
            );
        }
        assert_eq!(
            state.handle_editor_event(event::EXE, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        assert_eq!(state.error(), Some(NumericError::ColorOutOfRange));
        assert!(state.is_editing());
        assert_eq!(options.custom_rgb, original);

        assert_eq!(
            state.handle_editor_event(event::CLEAR, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        for value in [event::TWO, event::FIVE, event::FIVE] {
            assert_eq!(
                state.handle_editor_event(value, Domain::DEFAULT),
                SettingsAction::Redraw
            );
        }
        assert_eq!(
            state.handle_editor_event(event::EXE, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        assert!(!state.is_editing());
        assert_eq!(state.custom_color_draft().red, 255);
        assert_eq!(options.custom_rgb, original);

        assert_eq!(state.select_next(), SettingsAction::Redraw);
        assert_eq!(state.select_next(), SettingsAction::Redraw);
        assert_eq!(state.select_next(), SettingsAction::Redraw);
        assert_eq!(state.selected_custom_color_item(), CustomColorItem::Apply);
        assert_eq!(
            state.activate(&mut options, Domain::DEFAULT),
            SettingsAction::GraphChanged
        );
        assert_eq!(state.page(), SettingsPage::Appearance);
        assert_eq!(options.custom_rgb.red, 255);
        assert_eq!(options.custom_rgb.green, original.green);
        assert_eq!(options.custom_rgb.blue, original.blue);
        assert_eq!(options.surface_palette, SurfacePalette::Custom);
    }

    #[test]
    fn custom_color_back_cancels_channel_then_whole_draft() {
        let mut state = SettingsState::new();
        let mut options = GraphOptions::DEFAULT;
        let original = options.custom_rgb;
        enter_custom_color(&mut state, &mut options);
        assert_eq!(state.adjust_left(&mut options), SettingsAction::Redraw);
        assert_ne!(state.custom_color_draft(), original);

        assert_eq!(
            state.activate(&mut options, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        assert!(state.is_editing());
        assert_eq!(state.back(), SettingsAction::Redraw);
        assert!(!state.is_editing());
        assert_eq!(state.page(), SettingsPage::CustomColor);
        assert_eq!(state.back(), SettingsAction::Redraw);
        assert_eq!(state.page(), SettingsPage::Appearance);
        assert_eq!(options.custom_rgb, original);
    }

    #[test]
    fn color_channel_parser_accepts_only_zero_through_255() {
        assert_eq!(parse_color_channel(b"0"), Some(0));
        assert_eq!(parse_color_channel(b"255"), Some(255));
        assert_eq!(parse_color_channel(b"00128"), Some(128));
        assert_eq!(parse_color_channel(b""), None);
        assert_eq!(parse_color_channel(b"256"), None);
        assert_eq!(parse_color_channel(b"-1"), None);
        assert_eq!(parse_color_channel(b"1.0"), None);
    }

    #[test]
    fn settings_state_storage_remains_bounded() {
        assert_eq!(core::mem::size_of::<SettingsState>(), 80);
    }

    #[test]
    fn reset_camera_is_an_external_action() {
        let mut state = SettingsState::new();
        let mut options = GraphOptions::DEFAULT;
        move_to(&mut state, SettingsItem::ResetCamera);
        assert_eq!(
            state.activate(&mut options, Domain::DEFAULT),
            SettingsAction::ResetCamera
        );
        assert_eq!(state.selected_item(), SettingsItem::ResetCamera);
    }

    #[test]
    fn performance_readout_toggle_is_ui_only() {
        let mut state = SettingsState::new();
        let mut options = GraphOptions::DEFAULT;
        move_to(&mut state, SettingsItem::Performance);
        assert!(!options.show_performance);
        assert_eq!(state.adjust_right(&mut options), SettingsAction::Redraw);
        assert!(options.show_performance);
        assert_eq!(state.adjust_left(&mut options), SettingsAction::Redraw);
        assert!(!options.show_performance);
        assert_eq!(
            state.activate(&mut options, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        assert!(options.show_performance);
    }

    #[test]
    fn valid_domain_edit_is_returned_transactionally() {
        let mut state = SettingsState::new();
        let mut options = GraphOptions::DEFAULT;
        enter_domain(&mut state, &mut options);
        replace_edit_text(&mut state, Domain::DEFAULT, &[event::MINUS, event::TWO]);
        let action = state.handle_editor_event(event::EXE, Domain::DEFAULT);
        let candidate = match action {
            SettingsAction::DomainChanged(domain) => domain,
            _ => panic!("expected a valid domain"),
        };
        assert_eq!(candidate.x_min, -2.0);
        assert_eq!(candidate.x_max, Domain::DEFAULT.x_max);
        assert!(!state.is_editing());
        assert_eq!(state.page(), SettingsPage::Domain);
    }

    #[test]
    fn accepting_an_unchanged_formatted_bound_does_not_round_the_domain() {
        let mut state = SettingsState::new();
        let mut options = GraphOptions::DEFAULT;
        enter_domain(&mut state, &mut options);
        assert_eq!(
            state.activate(&mut options, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        assert_eq!(state.edit_source(), "-3.141593");
        assert_eq!(
            state.handle_editor_event(event::EXE, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        assert!(!state.is_editing());
        assert_eq!(Domain::DEFAULT.x_min, -3.1415927);
    }

    #[test]
    fn invalid_domain_keeps_the_active_value_and_editor_text() {
        let mut state = SettingsState::new();
        let mut options = GraphOptions::DEFAULT;
        enter_domain(&mut state, &mut options);
        replace_edit_text(&mut state, Domain::DEFAULT, &[event::ONE, event::ZERO]);
        assert_eq!(
            state.handle_editor_event(event::EXE, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        assert_eq!(
            state.error(),
            Some(NumericError::Domain(DomainError::Inverted))
        );
        assert_eq!(state.edit_source(), "10");
        assert!(state.is_editing());
        assert_eq!(Domain::DEFAULT.x_min, -3.1415927);
    }

    #[test]
    fn malformed_number_is_rejected_without_leaving_edit_mode() {
        let mut state = SettingsState::new();
        let mut options = GraphOptions::DEFAULT;
        enter_domain(&mut state, &mut options);
        replace_edit_text(
            &mut state,
            Domain::DEFAULT,
            &[event::ONE, event::DOT, event::DOT, event::TWO],
        );
        assert_eq!(
            state.handle_editor_event(event::EXE, Domain::DEFAULT),
            SettingsAction::Redraw
        );
        assert_eq!(state.error(), Some(NumericError::InvalidNumber));
        assert!(state.is_editing());
    }

    #[test]
    fn back_cancels_edit_then_navigates_outward() {
        let mut state = SettingsState::new();
        let mut options = GraphOptions::DEFAULT;
        enter_domain(&mut state, &mut options);
        replace_edit_text(&mut state, Domain::DEFAULT, &[event::TWO]);
        assert_eq!(state.back(), SettingsAction::Redraw);
        assert!(!state.is_editing());
        assert_eq!(state.page(), SettingsPage::Domain);
        assert_eq!(state.back(), SettingsAction::Redraw);
        assert_eq!(state.page(), SettingsPage::Main);
        assert_eq!(state.back(), SettingsAction::LeaveSettings);
    }

    #[test]
    fn numeric_parser_supports_sign_decimal_and_exponent() {
        assert_eq!(parse_number(b".5"), Some(0.5));
        assert_eq!(parse_number(b"-2.25e+1"), Some(-22.5));
        assert_eq!(parse_number(b"+3e-1"), Some(0.3));
        assert_eq!(parse_number(b""), None);
        assert_eq!(parse_number(b"--1"), None);
        assert_eq!(parse_number(b"1e"), None);
        assert_eq!(parse_number(b"1e39"), None);
    }

    #[test]
    fn number_format_is_fixed_capacity_and_c_terminated() {
        let pi = NumberText::new(-3.1415927);
        assert_eq!(pi.as_bytes(), b"-3.141593");
        assert_eq!(pi.as_c_string().last(), Some(&0));
        assert!(pi.as_bytes().len() <= NUMERIC_CAPACITY);
        assert_eq!(NumberText::new(12.5).as_bytes(), b"12.5");
    }

    #[test]
    fn domain_selection_is_bounded() {
        let mut state = SettingsState::new();
        let mut options = GraphOptions::DEFAULT;
        enter_domain(&mut state, &mut options);
        assert_eq!(state.select_previous(), SettingsAction::None);
        assert_eq!(state.selected_domain_field(), DomainField::XMin);
        assert_eq!(state.select_next(), SettingsAction::Redraw);
        assert_eq!(state.select_next(), SettingsAction::Redraw);
        assert_eq!(state.select_next(), SettingsAction::Redraw);
        assert_eq!(state.selected_domain_field(), DomainField::YMax);
        assert_eq!(state.select_next(), SettingsAction::None);
    }
}

//! Application-level tab/focus state machine and dirty-state ownership.
//!
//! There are four user interaction contexts: Graph content, Equation content,
//! Settings content, and the shared tab bar. Content OK moves focus to the tab
//! bar; tab Left/Right changes selection; tab OK atomically activates content;
//! tab Back cancels navigation. Equation Back returns to Graph without compiling,
//! while Graph Back exits. These transitions stay in the cooperative main loop—
//! no tab or editor owns a nested/blocking input loop.
//!
//! Raw keyboard state owns application transitions and continuous graph controls.
//! Semantic EADK events own calculator-style editor characters. Keeping those
//! responsibilities separate prevents a physical key from being applied twice.

use crate::camera::Camera;
use crate::eadk::keyboard;
use crate::editor::EditorAction;
use crate::editor::EditorKeyRepeat;
use crate::editor::EquationEditor;
use crate::expression::CompiledExpression;
use crate::functions::{self, pair_mask_for_function, MAX_FUNCTIONS, MAX_FUNCTION_PAIRS};
use crate::graph::GraphOptions;
use crate::input;
use crate::settings::{parse_color_channel, NumericEditor, SettingsAction, SettingsState};
use crate::surface::Domain;

#[derive(Clone, Copy, Debug, PartialEq)]
/// Top-level application views.
pub enum Tab {
    Graph,
    Equation,
    Settings,
}

impl Tab {
    /// Stable display index used by the three-segment header.
    pub fn index(self) -> usize {
        match self {
            Tab::Graph => 0,
            Tab::Equation => 1,
            Tab::Settings => 2,
        }
    }

    fn previous(self) -> Tab {
        match self {
            Tab::Graph => Tab::Settings,
            Tab::Equation => Tab::Graph,
            Tab::Settings => Tab::Equation,
        }
    }

    fn next(self) -> Tab {
        match self {
            Tab::Graph => Tab::Equation,
            Tab::Equation => Tab::Settings,
            Tab::Settings => Tab::Graph,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// Whether input belongs to the selected tab's content or to tab navigation.
pub enum Focus {
    Content,
    Tabs,
}

/// Independent invalidation domains.
///
/// `content` means the current tab body needs drawing. `graph` durably records
/// that the graph viewport must be recomposed, even when Settings is currently
/// covering it. `surface` specifically means equation/domain heights must be
/// resampled. `header` is isolated so camera motion never redraws static UI.
pub struct DirtyFlags {
    pub header: bool,
    pub content: bool,
    pub graph: bool,
    pub surface: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EquationPage {
    FunctionList,
    FunctionDetail,
    ExpressionEditor,
    CustomColor,
    Intersections,
}

#[derive(Clone, Copy, PartialEq)]
/// Outcome consumed by the cooperative main loop.
pub enum UpdateResult {
    Continue,
    StateChanged,
    Exit,
}

/// Complete allocation-free application/UI state.
pub struct AppState {
    pub camera: Camera,
    pub domain: Domain,
    pub graph_options: GraphOptions,
    pub settings: SettingsState,
    pub active_tab: Tab,
    pub selected_tab: Tab,
    pub focus: Focus,
    pub dirty: DirtyFlags,
    pub editor: EquationEditor,
    pub equation_page: EquationPage,
    pub selected_function: u8,
    pub function_detail_row: u8,
    pub selected_pair: u8,
    pub surface_dirty_mask: u8,
    pub intersection_dirty_mask: u8,
    pub coordinates_dirty: bool,
    pub custom_color_draft: crate::graph::Rgb888,
    pub custom_color_row: u8,
    pub custom_numeric_editing: bool,
    pub custom_numeric_error: bool,
    pub(crate) custom_numeric: NumericEditor,
    auto_rotate: bool,
    auto_rotate_armed: bool,
    auto_rotate_last_ms: u64,
    manual_camera_changed: bool,
    last_graph_render_ms: u32,
    editor_repeat: EditorKeyRepeat,
    previous_keys: keyboard::State,
    pressed_keys: keyboard::State,
}

impl AppState {
    /// Starts in Graph content with the default domain and equation editor text.
    pub fn new() -> AppState {
        AppState {
            camera: Camera::new(),
            domain: Domain::DEFAULT,
            graph_options: GraphOptions::DEFAULT,
            settings: SettingsState::new(),
            active_tab: Tab::Graph,
            selected_tab: Tab::Graph,
            focus: Focus::Content,
            dirty: DirtyFlags {
                header: true,
                content: true,
                graph: true,
                surface: true,
            },
            editor: EquationEditor::new(),
            equation_page: EquationPage::FunctionList,
            selected_function: 0,
            function_detail_row: 0,
            selected_pair: 0,
            surface_dirty_mask: 1,
            intersection_dirty_mask: 0,
            coordinates_dirty: false,
            custom_color_draft: crate::graph::Rgb888::DEFAULT_CUSTOM,
            custom_color_row: 0,
            custom_numeric_editing: false,
            custom_numeric_error: false,
            custom_numeric: NumericEditor::new(),
            auto_rotate: false,
            auto_rotate_armed: false,
            auto_rotate_last_ms: 0,
            manual_camera_changed: false,
            last_graph_render_ms: 0,
            editor_repeat: EditorKeyRepeat::new(),
            previous_keys: 0,
            pressed_keys: 0,
        }
    }

    fn mark_function_dirty(&mut self, function: usize) {
        if function < MAX_FUNCTIONS {
            self.surface_dirty_mask |= 1 << function;
            self.intersection_dirty_mask |= pair_mask_for_function(function);
            self.dirty.surface = true;
            self.dirty.graph = true;
        }
    }

    fn mark_all_enabled_surfaces_dirty(&mut self) {
        let enabled = functions::with_active_functions(|set| set.enabled_mask());
        self.surface_dirty_mask |= enabled;
        self.intersection_dirty_mask = (1 << MAX_FUNCTION_PAIRS) - 1;
        self.dirty.surface = self.surface_dirty_mask != 0;
        self.dirty.graph = true;
    }

    pub fn handle_equation_event(&mut self, event: crate::eadk::event::Event) -> UpdateResult {
        use crate::eadk::event as key;
        match self.equation_page {
            EquationPage::FunctionList => match event {
                key::UP => {
                    self.selected_function = self.selected_function.saturating_sub(1);
                    self.dirty.content = true;
                    UpdateResult::Continue
                }
                key::DOWN => {
                    self.selected_function = (self.selected_function + 1).min(3);
                    self.dirty.content = true;
                    UpdateResult::Continue
                }
                key::EXE => {
                    self.equation_page = EquationPage::FunctionDetail;
                    self.function_detail_row = 0;
                    self.dirty.content = true;
                    UpdateResult::StateChanged
                }
                key::TOOLBOX => {
                    self.equation_page = EquationPage::Intersections;
                    self.dirty.content = true;
                    UpdateResult::StateChanged
                }
                key::OK => self.focus_equation_tabs(),
                key::BACK => {
                    self.show_graph();
                    UpdateResult::StateChanged
                }
                _ => UpdateResult::Continue,
            },
            EquationPage::FunctionDetail => self.handle_function_detail_event(event),
            EquationPage::ExpressionEditor => match self.editor.handle_event(event) {
                EditorAction::None => UpdateResult::Continue,
                EditorAction::Changed => {
                    self.save_selected_draft();
                    self.dirty.content = true;
                    UpdateResult::Continue
                }
                EditorAction::Submit => {
                    self.save_selected_draft();
                    let index = self.selected_function as usize;
                    let compiled = functions::with_active_functions(|set| {
                        set.slots[index].compile_draft().is_ok()
                    });
                    if compiled {
                        self.editor.dismiss_error();
                        self.mark_function_dirty(index);
                        self.show_graph();
                        UpdateResult::StateChanged
                    } else {
                        // Populate the editor's structured parse error without
                        // changing the slot's last valid bytecode.
                        let mut temporary = match CompiledExpression::compile("x") {
                            Ok(value) => value,
                            Err(_) => return UpdateResult::Continue,
                        };
                        let _ = self.editor.compile_into(&mut temporary);
                        self.dirty.content = true;
                        UpdateResult::Continue
                    }
                }
                EditorAction::Cancel => {
                    self.save_selected_draft();
                    self.editor.dismiss_error();
                    self.equation_page = EquationPage::FunctionDetail;
                    self.dirty.content = true;
                    UpdateResult::StateChanged
                }
                EditorAction::FocusTabs => self.focus_equation_tabs(),
            },
            EquationPage::Intersections => match event {
                key::UP => {
                    self.selected_pair = self.selected_pair.saturating_sub(1);
                    self.dirty.content = true;
                    UpdateResult::Continue
                }
                key::DOWN => {
                    self.selected_pair = (self.selected_pair + 1).min(5);
                    self.dirty.content = true;
                    UpdateResult::Continue
                }
                key::LEFT | key::RIGHT | key::EXE => {
                    crate::intersections::with_intersections(|cache| {
                        cache.toggle_visibility(self.selected_pair as usize)
                    });
                    self.dirty.content = true;
                    self.dirty.graph = true;
                    UpdateResult::Continue
                }
                key::BACK => {
                    self.equation_page = EquationPage::FunctionList;
                    self.dirty.content = true;
                    UpdateResult::StateChanged
                }
                key::OK => self.focus_equation_tabs(),
                _ => UpdateResult::Continue,
            },
            EquationPage::CustomColor => self.handle_custom_color_event(event),
        }
    }

    fn handle_function_detail_event(&mut self, event: crate::eadk::event::Event) -> UpdateResult {
        use crate::eadk::event as key;
        match event {
            key::UP => self.function_detail_row = self.function_detail_row.saturating_sub(1),
            key::DOWN => self.function_detail_row = (self.function_detail_row + 1).min(2),
            key::BACK => {
                self.equation_page = EquationPage::FunctionList;
                self.dirty.content = true;
                return UpdateResult::StateChanged;
            }
            key::OK => return self.focus_equation_tabs(),
            key::LEFT | key::RIGHT | key::EXE => {
                let index = self.selected_function as usize;
                if self.function_detail_row == 0 {
                    let can_enable = functions::with_active_functions(|set| {
                        if set.slots[index].enabled {
                            set.slots[index].enabled = false;
                            true
                        } else if set.slots[index].can_enable() {
                            set.slots[index].enabled = true;
                            true
                        } else {
                            false
                        }
                    });
                    if can_enable {
                        self.mark_function_dirty(index);
                    } else {
                        self.open_expression_editor();
                    }
                } else if self.function_detail_row == 1 {
                    if event == key::EXE {
                        self.open_expression_editor();
                    }
                } else {
                    let current = functions::with_active_functions(|set| set.slots[index].palette);
                    if event == key::EXE && current == crate::graph::SurfacePalette::Custom {
                        self.custom_color_draft =
                            functions::with_active_functions(|set| set.slots[index].custom_rgb);
                        self.custom_color_row = 0;
                        self.equation_page = EquationPage::CustomColor;
                    } else {
                        functions::with_active_functions(|set| {
                            let slot = &mut set.slots[index];
                            slot.palette = if event == key::LEFT {
                                slot.palette.previous()
                            } else {
                                slot.palette.next()
                            };
                        });
                    }
                    self.dirty.graph = true;
                }
            }
            _ => {}
        }
        self.dirty.content = true;
        UpdateResult::Continue
    }

    fn handle_custom_color_event(&mut self, event: crate::eadk::event::Event) -> UpdateResult {
        use crate::eadk::event as key;
        if self.custom_numeric_editing {
            match event {
                key::LEFT => {
                    let _ = self.custom_numeric.move_left();
                }
                key::RIGHT => {
                    let _ = self.custom_numeric.move_right();
                }
                key::SHIFT_LEFT => {
                    let _ = self.custom_numeric.move_to_start();
                }
                key::SHIFT_RIGHT => {
                    let _ = self.custom_numeric.move_to_end();
                }
                key::BACKSPACE => {
                    let _ = self.custom_numeric.backspace();
                }
                key::CLEAR => {
                    let _ = self.custom_numeric.clear();
                }
                key::BACK => {
                    self.custom_numeric_editing = false;
                    self.custom_numeric_error = false;
                    self.custom_numeric.reset();
                }
                key::EXE => {
                    if let Some(value) =
                        parse_color_channel(self.custom_numeric.source().as_bytes())
                    {
                        match self.custom_color_row {
                            0 => self.custom_color_draft.red = value,
                            1 => self.custom_color_draft.green = value,
                            _ => self.custom_color_draft.blue = value,
                        }
                        self.custom_numeric_editing = false;
                        self.custom_numeric_error = false;
                        self.custom_numeric.reset();
                    } else {
                        self.custom_numeric_error = true;
                    }
                }
                key::ZERO => {
                    let _ = self.custom_numeric.insert(b"0");
                }
                key::ONE => {
                    let _ = self.custom_numeric.insert(b"1");
                }
                key::TWO => {
                    let _ = self.custom_numeric.insert(b"2");
                }
                key::THREE => {
                    let _ = self.custom_numeric.insert(b"3");
                }
                key::FOUR => {
                    let _ = self.custom_numeric.insert(b"4");
                }
                key::FIVE => {
                    let _ = self.custom_numeric.insert(b"5");
                }
                key::SIX => {
                    let _ = self.custom_numeric.insert(b"6");
                }
                key::SEVEN => {
                    let _ = self.custom_numeric.insert(b"7");
                }
                key::EIGHT => {
                    let _ = self.custom_numeric.insert(b"8");
                }
                key::NINE => {
                    let _ = self.custom_numeric.insert(b"9");
                }
                _ => {}
            }
            self.dirty.content = true;
            return UpdateResult::Continue;
        }
        match event {
            key::UP => self.custom_color_row = self.custom_color_row.saturating_sub(1),
            key::DOWN => self.custom_color_row = (self.custom_color_row + 1).min(3),
            key::LEFT | key::RIGHT if self.custom_color_row < 3 => {
                let channel = match self.custom_color_row {
                    0 => &mut self.custom_color_draft.red,
                    1 => &mut self.custom_color_draft.green,
                    _ => &mut self.custom_color_draft.blue,
                };
                *channel = if event == key::RIGHT {
                    channel.saturating_add(8)
                } else {
                    channel.saturating_sub(8)
                };
            }
            key::EXE if self.custom_color_row < 3 => {
                let value = match self.custom_color_row {
                    0 => self.custom_color_draft.red,
                    1 => self.custom_color_draft.green,
                    _ => self.custom_color_draft.blue,
                };
                self.custom_numeric.load(value as f32);
                self.custom_numeric_editing = true;
                self.custom_numeric_error = false;
            }
            key::EXE if self.custom_color_row == 3 => {
                let index = self.selected_function as usize;
                functions::with_active_functions(|set| {
                    set.slots[index].custom_rgb = self.custom_color_draft;
                    set.slots[index].palette = crate::graph::SurfacePalette::Custom;
                });
                self.equation_page = EquationPage::FunctionDetail;
                self.dirty.graph = true;
            }
            key::BACK => {
                self.custom_numeric.reset();
                self.custom_numeric_editing = false;
                self.equation_page = EquationPage::FunctionDetail;
            }
            key::OK => return self.focus_equation_tabs(),
            _ => {}
        }
        self.dirty.content = true;
        UpdateResult::Continue
    }

    fn open_expression_editor(&mut self) {
        let source = functions::with_active_functions(|set| {
            let slot = &set.slots[self.selected_function as usize];
            let mut copy = [0; crate::expression::MAX_EXPRESSION_LENGTH];
            let length = slot.draft_length as usize;
            copy[..length].copy_from_slice(slot.draft());
            (copy, length)
        });
        self.editor.load_source(&source.0[..source.1]);
        self.equation_page = EquationPage::ExpressionEditor;
    }

    fn save_selected_draft(&mut self) {
        let source = self.editor.source();
        functions::with_active_functions(|set| {
            let slot = &mut set.slots[self.selected_function as usize];
            let changed = slot.draft() != source.as_bytes();
            slot.draft_length = self.editor.copy_source_into(&mut slot.draft_source);
            // Merely opening and closing an editor must not make an unchanged
            // source look unapplied. Once bytes differ from the last known
            // draft, only a successful compile may re-establish the invariant.
            if changed {
                slot.draft_matches_compiled = false;
            }
        });
    }

    fn focus_equation_tabs(&mut self) -> UpdateResult {
        if self.equation_page == EquationPage::ExpressionEditor {
            self.save_selected_draft();
            let _ = self.editor.close_function_picker();
        }
        self.selected_tab = self.active_tab;
        self.focus = Focus::Tabs;
        self.dirty.header = true;
        self.dirty.content = true;
        UpdateResult::StateChanged
    }

    /// Whether transient automatic horizontal orbit is enabled.
    pub fn auto_rotate_enabled(&self) -> bool {
        self.auto_rotate
    }

    /// Advances automatic yaw using a bounded, frame-rate-independent interval.
    pub fn advance_auto_rotate(&mut self, now_ms: u64) {
        if self.manual_camera_changed {
            self.manual_camera_changed = false;
            self.auto_rotate_armed = false;
        }
        if !self.auto_rotate || self.active_tab != Tab::Graph || self.focus != Focus::Content {
            self.auto_rotate_armed = false;
            return;
        }
        if !self.auto_rotate_armed {
            self.auto_rotate_last_ms = now_ms;
            self.auto_rotate_armed = true;
            return;
        }
        let delta_ms = now_ms.saturating_sub(self.auto_rotate_last_ms).min(100);
        self.auto_rotate_last_ms = now_ms;
        if delta_ms == 0 {
            return;
        }
        self.camera
            .orbit(core::f32::consts::PI / 6.0 * delta_ms as f32 * 0.001, 0.0);
        self.dirty.content = true;
        self.dirty.graph = true;
    }

    /// Raw down edges discovered during the most recent `update` call.
    pub fn pressed_keys(&self) -> keyboard::State {
        self.pressed_keys
    }

    /// Duration of the latest complete graph redraw, including display transfer.
    pub fn graph_render_profile_ms(&self) -> u32 {
        self.last_graph_render_ms
    }

    /// Records one complete graph render for the temporary hardware profiler.
    pub fn record_graph_render_ms(&mut self, elapsed_ms: u64) {
        self.last_graph_render_ms = elapsed_ms.min(u32::MAX as u64) as u32;
    }

    /// Atomically returns to Graph content and invalidates its viewport/header.
    pub fn show_graph(&mut self) {
        self.active_tab = Tab::Graph;
        self.selected_tab = Tab::Graph;
        self.focus = Focus::Content;
        self.dirty.header = true;
        self.dirty.content = true;
        self.dirty.graph = true;
    }

    /// Applies one semantic calculator event to Equation content.
    ///
    /// Successful EXE replaces the active bytecode and marks surface samples
    /// dirty. Failed compilation leaves the active bytecode untouched.
    #[cfg(test)]
    pub fn handle_editor_event(
        &mut self,
        event: crate::eadk::event::Event,
        active_expression: &mut CompiledExpression,
    ) -> UpdateResult {
        match self.editor.handle_event(event) {
            EditorAction::None => UpdateResult::Continue,
            EditorAction::Changed => {
                self.dirty.content = true;
                UpdateResult::Continue
            }
            EditorAction::Submit => {
                if self.editor.compile_into(active_expression) {
                    self.dirty.surface = true;
                    self.show_graph();
                    UpdateResult::StateChanged
                } else {
                    self.dirty.content = true;
                    UpdateResult::Continue
                }
            }
            EditorAction::Cancel => {
                self.editor.dismiss_error();
                self.show_graph();
                UpdateResult::StateChanged
            }
            EditorAction::FocusTabs => {
                self.selected_tab = self.active_tab;
                self.focus = Focus::Tabs;
                self.dirty.header = true;
                self.dirty.content = true;
                UpdateResult::StateChanged
            }
        }
    }

    /// Generates bounded-time Backspace/Left/Right repeat events from raw held
    /// keys without blocking the main loop. The same timer is routed to whichever
    /// fixed-capacity text field currently owns semantic input.
    #[cfg(test)]
    pub fn update_key_repeat(
        &mut self,
        keys: keyboard::State,
        now_ms: u64,
        active_expression: &mut CompiledExpression,
    ) {
        let equation_active = self.active_tab == Tab::Equation && self.focus == Focus::Content;
        let settings_active = self.active_tab == Tab::Settings
            && self.focus == Focus::Content
            && self.settings.is_editing();
        if !equation_active && !settings_active {
            self.editor_repeat.reset();
            return;
        }
        if let Some(event) = self.editor_repeat.update(keys, now_ms) {
            if equation_active {
                let _ = self.handle_editor_event(event, active_expression);
            } else {
                let _ = self.handle_settings_event(event);
            }
        }
    }

    /// Repeat routing used by the multi-function Equation state machine.
    pub fn update_key_repeat_current(&mut self, keys: keyboard::State, now_ms: u64) {
        let equation_active = self.active_tab == Tab::Equation
            && self.focus == Focus::Content
            && (self.equation_page == EquationPage::ExpressionEditor
                || (self.equation_page == EquationPage::CustomColor
                    && self.custom_numeric_editing));
        let settings_active = self.active_tab == Tab::Settings
            && self.focus == Focus::Content
            && self.settings.is_editing();
        if !equation_active && !settings_active {
            self.editor_repeat.reset();
            return;
        }
        if let Some(event) = self.editor_repeat.update(keys, now_ms) {
            if equation_active {
                let _ = self.handle_equation_event(event);
            } else {
                let _ = self.handle_settings_event(event);
            }
        }
    }

    /// Applies one semantic numeric event to the active Settings domain field.
    pub fn handle_settings_event(&mut self, event: crate::eadk::event::Event) -> UpdateResult {
        let action = self.settings.handle_editor_event(event, self.domain);
        self.apply_settings_action(action)
    }

    /// Advances the focus/tab/camera state from one raw keyboard sample.
    pub fn update(&mut self, keys: keyboard::State) -> UpdateResult {
        let pressed = keys & !self.previous_keys;
        self.pressed_keys = pressed;
        self.previous_keys = keys;

        if keyboard::key_down(pressed, keyboard::BACK) {
            return self.handle_back();
        }

        if self.focus == Focus::Tabs {
            if keyboard::key_down(pressed, keyboard::LEFT) {
                self.selected_tab = self.selected_tab.previous();
                self.dirty.header = true;
            }
            if keyboard::key_down(pressed, keyboard::RIGHT) {
                self.selected_tab = self.selected_tab.next();
                self.dirty.header = true;
            }
            if keyboard::key_down(pressed, keyboard::OK) {
                if self.active_tab != self.selected_tab {
                    self.active_tab = self.selected_tab;
                    self.dirty.content = true;
                }
                self.focus = Focus::Content;
                self.dirty.header = true;
                self.dirty.content = true;
                return UpdateResult::StateChanged;
            }
            return UpdateResult::Continue;
        }

        if keyboard::key_down(pressed, keyboard::OK) {
            // Leaving a numeric field for the tab bar cancels its draft exactly
            // like Back; the active domain remains transactional and unchanged.
            if self.active_tab == Tab::Settings && self.settings.is_editing() {
                let _ = self.settings.back();
            }
            if self.active_tab == Tab::Equation {
                if self.equation_page == EquationPage::ExpressionEditor {
                    self.save_selected_draft();
                }
                if self.equation_page == EquationPage::CustomColor {
                    self.custom_numeric_editing = false;
                    self.custom_numeric_error = false;
                    self.custom_numeric.reset();
                }
                let _ = self.editor.close_function_picker();
            }
            self.selected_tab = self.active_tab;
            self.focus = Focus::Tabs;
            self.dirty.header = true;
            if self.active_tab == Tab::Equation || self.active_tab == Tab::Settings {
                self.dirty.content = true;
            }
            return UpdateResult::StateChanged;
        }

        if self.active_tab == Tab::Graph {
            if let input::Action::Redraw = input::update(&mut self.camera, keys) {
                self.manual_camera_changed = true;
                self.dirty.content = true;
                self.dirty.graph = true;
            }
        } else if self.active_tab == Tab::Settings && !self.settings.is_editing() {
            let action = if keyboard::key_down(pressed, keyboard::UP) {
                self.settings.select_previous()
            } else if keyboard::key_down(pressed, keyboard::DOWN) {
                self.settings.select_next()
            } else if keyboard::key_down(pressed, keyboard::LEFT) {
                self.settings.adjust_left(&mut self.graph_options)
            } else if keyboard::key_down(pressed, keyboard::RIGHT) {
                self.settings.adjust_right(&mut self.graph_options)
            } else if keyboard::key_down(pressed, keyboard::EXE) {
                self.settings.activate(&mut self.graph_options, self.domain)
            } else {
                SettingsAction::None
            };
            return self.apply_settings_action(action);
        }
        UpdateResult::Continue
    }

    fn handle_back(&mut self) -> UpdateResult {
        if self.focus == Focus::Tabs {
            self.selected_tab = self.active_tab;
            self.focus = Focus::Content;
            self.dirty.header = true;
            if self.active_tab == Tab::Equation || self.active_tab == Tab::Settings {
                self.dirty.content = true;
            }
            return UpdateResult::StateChanged;
        }
        if self.active_tab == Tab::Settings {
            let action = self.settings.back();
            return self.apply_settings_action(action);
        }
        if self.active_tab == Tab::Equation && self.editor.close_function_picker() {
            self.dirty.content = true;
            return UpdateResult::StateChanged;
        }
        if self.active_tab == Tab::Equation {
            if self.equation_page == EquationPage::CustomColor && self.custom_numeric_editing {
                self.custom_numeric_editing = false;
                self.custom_numeric_error = false;
                self.custom_numeric.reset();
                self.dirty.content = true;
                return UpdateResult::StateChanged;
            }
            match self.equation_page {
                EquationPage::ExpressionEditor => {
                    self.save_selected_draft();
                    self.editor.dismiss_error();
                    self.equation_page = EquationPage::FunctionDetail;
                }
                EquationPage::FunctionDetail
                | EquationPage::CustomColor
                | EquationPage::Intersections => {
                    self.equation_page = EquationPage::FunctionList;
                }
                EquationPage::FunctionList => {
                    self.show_graph();
                    return UpdateResult::StateChanged;
                }
            }
            self.dirty.content = true;
            return UpdateResult::StateChanged;
        }
        if self.active_tab != Tab::Graph {
            self.active_tab = Tab::Graph;
            self.selected_tab = Tab::Graph;
            self.dirty.header = true;
            self.dirty.content = true;
            return UpdateResult::StateChanged;
        }
        UpdateResult::Exit
    }

    fn apply_settings_action(&mut self, action: SettingsAction) -> UpdateResult {
        match action {
            SettingsAction::None => UpdateResult::Continue,
            SettingsAction::Redraw => {
                self.dirty.content = true;
                UpdateResult::Continue
            }
            SettingsAction::GraphChanged => {
                self.dirty.content = true;
                self.dirty.graph = true;
                UpdateResult::Continue
            }
            SettingsAction::ResolutionChanged => {
                self.dirty.content = true;
                self.dirty.graph = true;
                self.dirty.surface = true;
                self.coordinates_dirty = true;
                self.mark_all_enabled_surfaces_dirty();
                UpdateResult::Continue
            }
            SettingsAction::AutoRotateChanged => {
                self.auto_rotate = !self.auto_rotate;
                self.auto_rotate_armed = false;
                self.dirty.content = true;
                self.dirty.graph = true;
                UpdateResult::Continue
            }
            SettingsAction::DomainChanged(domain) => {
                self.domain = domain;
                self.dirty.content = true;
                self.dirty.graph = true;
                self.dirty.surface = true;
                self.coordinates_dirty = true;
                self.mark_all_enabled_surfaces_dirty();
                UpdateResult::Continue
            }
            SettingsAction::ResetCamera => {
                self.camera.reset();
                self.manual_camera_changed = true;
                self.dirty.content = true;
                self.dirty.graph = true;
                UpdateResult::Continue
            }
            SettingsAction::LeaveSettings => {
                self.show_graph();
                UpdateResult::StateChanged
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eadk::event;
    use crate::function::SurfaceFunction;

    fn key(key: u8) -> keyboard::State {
        1_u64 << key
    }

    fn release(app: &mut AppState) {
        let _ = app.update(0);
    }

    fn enter_equation(app: &mut AppState) {
        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.focus, Focus::Tabs);
        release(app);

        assert!(matches!(
            app.update(key(keyboard::RIGHT)),
            UpdateResult::Continue
        ));
        assert_eq!(app.selected_tab, Tab::Equation);
        release(app);

        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.active_tab, Tab::Equation);
        assert_eq!(app.focus, Focus::Content);
        release(app);
    }

    fn enter_settings(app: &mut AppState) {
        assert_eq!(app.graph_options, GraphOptions::DEFAULT);
        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        release(app);
        let _ = app.update(key(keyboard::RIGHT));
        release(app);
        let _ = app.update(key(keyboard::RIGHT));
        assert_eq!(app.selected_tab, Tab::Settings);
        release(app);
        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.active_tab, Tab::Settings);
        assert_eq!(app.focus, Focus::Content);
        release(app);
    }

    fn settings_move_down(app: &mut AppState, rows: usize) {
        let mut row = 0;
        while row < rows {
            let _ = app.update(key(keyboard::DOWN));
            release(app);
            row += 1;
        }
    }

    #[test]
    fn graph_tabs_and_equation_focus_transitions() {
        let mut app = AppState::new();
        enter_equation(&mut app);

        let before = app.editor.source().len();
        let mut active = CompiledExpression::compile("x").expect("valid expression");
        assert!(matches!(
            app.handle_editor_event(event::ONE, &mut active),
            UpdateResult::Continue
        ));
        assert_eq!(app.active_tab, Tab::Equation);
        assert_eq!(app.focus, Focus::Content);
        assert_eq!(app.editor.source().len(), before + 1);

        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.focus, Focus::Tabs);
    }

    #[test]
    fn exe_compiles_and_returns_to_graph() {
        let mut app = AppState::new();
        enter_equation(&mut app);
        let mut active = CompiledExpression::compile("x").expect("valid expression");
        app.dirty.surface = false;

        assert!(matches!(
            app.handle_editor_event(event::EXE, &mut active),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.active_tab, Tab::Graph);
        assert_eq!(app.focus, Focus::Content);
        assert!(app.dirty.surface);
        let expected = CompiledExpression::compile("sin(x) * cos(y)")
            .expect("valid expression")
            .evaluate(0.5, 0.25);
        assert!((active.evaluate(0.5, 0.25) - expected).abs() < 0.0001);
    }

    #[test]
    fn equation_back_preserves_active_expression() {
        let mut app = AppState::new();
        enter_equation(&mut app);
        let active = CompiledExpression::compile("x+y").expect("valid expression");
        let before = active.evaluate(2.0, 3.0);

        assert!(matches!(
            app.update(key(keyboard::BACK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.active_tab, Tab::Graph);
        assert_eq!(active.evaluate(2.0, 3.0), before);
    }

    #[test]
    fn equation_picker_back_and_ok_preserve_equation_focus_rules() {
        let mut app = AppState::new();
        enter_equation(&mut app);
        let mut active = CompiledExpression::compile("x").expect("valid expression");
        assert!(matches!(
            app.handle_editor_event(event::TOOLBOX, &mut active),
            UpdateResult::Continue
        ));
        assert!(app.editor.function_picker_open());
        assert!(matches!(
            app.update(key(keyboard::BACK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.active_tab, Tab::Equation);
        assert_eq!(app.focus, Focus::Content);
        assert!(!app.editor.function_picker_open());

        release(&mut app);
        let _ = app.handle_editor_event(event::TOOLBOX, &mut active);
        assert!(app.editor.function_picker_open());
        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.focus, Focus::Tabs);
        assert!(!app.editor.function_picker_open());
    }

    #[test]
    fn graph_back_exits() {
        let mut app = AppState::new();
        assert!(matches!(
            app.update(key(keyboard::BACK)),
            UpdateResult::Exit
        ));
    }

    #[test]
    fn app_state_storage_remains_bounded() {
        assert!(core::mem::size_of::<AppState>() <= 400);
    }

    #[test]
    fn auto_rotate_defaults_off_and_toggles_from_appearance() {
        let mut app = AppState::new();
        assert!(!app.auto_rotate_enabled());
        enter_settings(&mut app);
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        settings_move_down(&mut app, 3);
        app.dirty.surface = false;
        let _ = app.update(key(keyboard::EXE));
        assert!(app.auto_rotate_enabled());
        assert!(!app.dirty.surface);
        release(&mut app);
        let _ = app.update(key(keyboard::EXE));
        assert!(!app.auto_rotate_enabled());
    }

    #[test]
    fn auto_rotate_is_bounded_focus_aware_and_camera_only() {
        let mut app = AppState::new();
        app.auto_rotate = true;
        app.dirty.content = false;
        app.dirty.graph = false;
        app.dirty.surface = false;
        let original_pitch = app.camera.pitch;
        let original_distance = app.camera.distance;
        app.advance_auto_rotate(1_000);
        let initial_yaw = app.camera.yaw;
        app.advance_auto_rotate(2_000);
        let capped_yaw_delta = app.camera.yaw - initial_yaw;
        assert!((capped_yaw_delta - core::f32::consts::PI / 6.0 * 0.1).abs() < 0.0001);
        assert_eq!(app.camera.pitch, original_pitch);
        assert_eq!(app.camera.distance, original_distance);
        assert!(app.dirty.content);
        assert!(app.dirty.graph);
        assert!(!app.dirty.surface);

        app.active_tab = Tab::Equation;
        let paused_yaw = app.camera.yaw;
        app.advance_auto_rotate(3_000);
        app.active_tab = Tab::Graph;
        app.advance_auto_rotate(10_000);
        assert_eq!(app.camera.yaw, paused_yaw);
        app.advance_auto_rotate(10_050);
        assert!(app.camera.yaw != paused_yaw);
    }

    #[test]
    fn manual_camera_motion_rearms_auto_rotate_clock() {
        let mut app = AppState::new();
        app.auto_rotate = true;
        app.advance_auto_rotate(100);
        app.advance_auto_rotate(150);
        let _ = app.update(key(keyboard::RIGHT));
        let yaw_after_manual = app.camera.yaw;
        app.advance_auto_rotate(10_000);
        assert_eq!(app.camera.yaw, yaw_after_manual);
        app.advance_auto_rotate(10_050);
        assert!(app.camera.yaw != yaw_after_manual);
    }

    #[test]
    fn camera_motion_invalidates_projection_but_not_surface_samples() {
        let mut app = AppState::new();
        app.dirty.content = false;
        app.dirty.graph = false;
        app.dirty.surface = false;
        assert!(matches!(
            app.update(key(keyboard::RIGHT)),
            UpdateResult::Continue
        ));
        assert!(app.dirty.content);
        assert!(app.dirty.graph);
        assert!(!app.dirty.surface);
    }

    #[test]
    fn settings_mode_change_dirties_hidden_graph_but_not_surface() {
        let mut app = AppState::new();
        enter_settings(&mut app);
        app.dirty.graph = false;
        app.dirty.surface = false;
        app.dirty.content = false;

        assert!(matches!(
            app.update(key(keyboard::RIGHT)),
            UpdateResult::Continue
        ));
        assert_eq!(
            app.graph_options.rendering_mode,
            crate::graph::RenderingMode::Solid
        );
        assert!(app.dirty.graph);
        assert!(app.dirty.content);
        assert!(!app.dirty.surface);
    }

    #[test]
    fn settings_appearance_change_dirties_graph_without_resampling() {
        let mut app = AppState::new();
        enter_settings(&mut app);
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        assert_eq!(
            app.settings.page(),
            crate::settings::SettingsPage::Appearance
        );

        app.dirty.graph = false;
        app.dirty.surface = false;
        app.dirty.content = false;
        let _ = app.update(key(keyboard::RIGHT));
        assert_eq!(
            app.graph_options.lighting,
            crate::graph::LightingPreset::Soft
        );
        assert!(app.dirty.graph);
        assert!(app.dirty.content);
        assert!(!app.dirty.surface);

        release(&mut app);
        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.focus, Focus::Tabs);
        assert_eq!(
            app.settings.page(),
            crate::settings::SettingsPage::Appearance
        );
    }

    #[test]
    fn resolution_change_dirties_surface_and_graph_without_navigation_resampling() {
        let mut app = AppState::new();
        enter_settings(&mut app);
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        settings_move_down(&mut app, 1);
        app.dirty.graph = false;
        app.dirty.surface = false;
        app.dirty.content = false;
        let _ = app.update(key(keyboard::RIGHT));
        assert_eq!(
            app.graph_options.resolution,
            crate::surface::ResolutionPreset::High
        );
        assert!(app.dirty.graph);
        assert!(app.dirty.surface);
        assert!(app.dirty.content);
    }

    #[test]
    fn ultra_resolution_change_keeps_surface_dirty_and_auto_rotate_camera_only() {
        let mut app = AppState::new();
        enter_settings(&mut app);
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        settings_move_down(&mut app, 1);
        let _ = app.update(key(keyboard::RIGHT));
        release(&mut app);
        let _ = app.update(key(keyboard::RIGHT));
        assert_eq!(
            app.graph_options.resolution,
            crate::surface::ResolutionPreset::Ultra
        );
        assert!(app.dirty.surface);

        app.dirty.surface = false;
        app.auto_rotate = true;
        app.show_graph();
        app.advance_auto_rotate(100);
        app.advance_auto_rotate(150);
        assert!(!app.dirty.surface);
        assert!(app.dirty.graph);
    }

    #[test]
    fn custom_color_apply_dirties_graph_without_resampling() {
        let mut app = AppState::new();
        app.active_tab = Tab::Equation;
        app.equation_page = EquationPage::CustomColor;
        app.custom_color_draft = crate::graph::Rgb888::DEFAULT_CUSTOM;
        let original = app.custom_color_draft;
        let _ = app.handle_equation_event(event::RIGHT);
        let _ = app.handle_equation_event(event::DOWN);
        let _ = app.handle_equation_event(event::DOWN);
        let _ = app.handle_equation_event(event::DOWN);
        app.dirty.content = false;
        app.dirty.graph = false;
        app.dirty.surface = false;
        let _ = app.handle_equation_event(event::EXE);
        let applied = functions::with_active_functions(|set| set.slots[0].custom_rgb);
        assert_eq!(applied.red, original.red + 8);
        assert!(app.dirty.content);
        assert!(app.dirty.graph);
        assert!(!app.dirty.surface);
    }

    #[test]
    fn custom_color_ok_focuses_tabs_without_committing_draft() {
        let mut app = AppState::new();
        app.active_tab = Tab::Equation;
        app.equation_page = EquationPage::CustomColor;
        let original = functions::with_active_functions(|set| set.slots[0].custom_rgb);
        let _ = app.handle_equation_event(event::RIGHT);
        release(&mut app);

        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.focus, Focus::Tabs);
        assert_eq!(app.equation_page, EquationPage::CustomColor);
        assert_eq!(
            functions::with_active_functions(|set| set.slots[0].custom_rgb),
            original
        );
    }

    #[test]
    fn settings_camera_reset_invalidates_projection_only() {
        let mut app = AppState::new();
        app.camera.orbit(0.4, -0.2);
        app.camera.truck(1.0);
        enter_settings(&mut app);
        settings_move_down(&mut app, 6);
        app.dirty.graph = false;
        app.dirty.surface = false;

        let _ = app.update(key(keyboard::EXE));
        let default = Camera::new();
        assert_eq!(app.camera.yaw, default.yaw);
        assert_eq!(app.camera.pitch, default.pitch);
        assert_eq!(app.camera.target_x, default.target_x);
        assert_eq!(app.camera.target_y, default.target_y);
        assert!(app.dirty.graph);
        assert!(!app.dirty.surface);
    }

    #[test]
    fn accepted_settings_domain_is_transactional_and_resamples() {
        let mut app = AppState::new();
        enter_settings(&mut app);
        settings_move_down(&mut app, 5);
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        assert_eq!(app.settings.page(), crate::settings::SettingsPage::Domain);
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        assert!(app.settings.is_editing());

        let _ = app.handle_settings_event(event::CLEAR);
        let _ = app.handle_settings_event(event::MINUS);
        let _ = app.handle_settings_event(event::TWO);
        app.dirty.graph = false;
        app.dirty.surface = false;
        let _ = app.handle_settings_event(event::EXE);

        assert_eq!(app.domain.x_min, -2.0);
        assert!(app.dirty.graph);
        assert!(app.dirty.surface);
        assert!(!app.settings.is_editing());
    }

    #[test]
    fn settings_back_unwinds_edit_page_then_returns_to_graph() {
        let mut app = AppState::new();
        enter_settings(&mut app);
        settings_move_down(&mut app, 5);
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        assert!(app.settings.is_editing());

        assert!(matches!(
            app.update(key(keyboard::BACK)),
            UpdateResult::Continue
        ));
        assert!(!app.settings.is_editing());
        release(&mut app);
        assert!(matches!(
            app.update(key(keyboard::BACK)),
            UpdateResult::Continue
        ));
        assert_eq!(app.settings.page(), crate::settings::SettingsPage::Main);
        release(&mut app);
        assert!(matches!(
            app.update(key(keyboard::BACK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.active_tab, Tab::Graph);
        assert_eq!(app.focus, Focus::Content);
    }

    #[test]
    fn settings_tab_back_redraws_the_restored_content_focus() {
        let mut app = AppState::new();
        enter_settings(&mut app);
        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.focus, Focus::Tabs);
        release(&mut app);

        // Model the main loop having drawn the unfocused Settings content.
        app.dirty.content = false;
        assert!(matches!(
            app.update(key(keyboard::BACK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.focus, Focus::Content);
        assert!(app.dirty.content);
    }

    #[test]
    fn empty_function_opens_editor_and_compiles_transactionally() {
        functions::reset_active_functions();
        let mut app = AppState::new();
        app.active_tab = Tab::Equation;
        app.selected_function = 1;
        app.equation_page = EquationPage::FunctionDetail;
        app.function_detail_row = 0;
        app.surface_dirty_mask = 0;
        app.intersection_dirty_mask = 0;

        let _ = app.handle_equation_event(event::EXE);
        assert_eq!(app.equation_page, EquationPage::ExpressionEditor);
        assert_eq!(app.editor.source(), "");
        let _ = app.handle_equation_event(event::XNT);
        let _ = app.handle_equation_event(event::EXE);
        assert_eq!(app.active_tab, Tab::Graph);
        assert_eq!(app.surface_dirty_mask, 0b0010);
        assert_eq!(app.intersection_dirty_mask, pair_mask_for_function(1));
        functions::with_active_functions(|set| {
            assert!(set.slots[1].enabled);
            assert!(set.slots[1].compiled.is_some());
            assert_eq!(set.slots[1].draft(), b"x");
        });
    }

    #[test]
    fn function_color_and_pair_visibility_are_render_only_changes() {
        functions::reset_active_functions();
        crate::intersections::with_intersections(|cache| cache.initialize());
        let mut app = AppState::new();
        app.active_tab = Tab::Equation;
        app.equation_page = EquationPage::FunctionDetail;
        app.function_detail_row = 2;
        app.dirty.surface = false;
        app.surface_dirty_mask = 0;
        let before = functions::with_active_functions(|set| set.slots[0].palette);
        let _ = app.handle_equation_event(event::RIGHT);
        let after = functions::with_active_functions(|set| set.slots[0].palette);
        assert_ne!(before, after);
        assert!(app.dirty.graph);
        assert!(!app.dirty.surface);
        assert_eq!(app.surface_dirty_mask, 0);

        app.equation_page = EquationPage::Intersections;
        app.selected_pair = 0;
        app.dirty.graph = false;
        let before_visibility =
            crate::intersections::with_intersections(|cache| cache.visibility_mask());
        let _ = app.handle_equation_event(event::EXE);
        let after_visibility =
            crate::intersections::with_intersections(|cache| cache.visibility_mask());
        assert_eq!(after_visibility, before_visibility ^ 1);
        assert!(app.dirty.graph);
        assert_eq!(app.surface_dirty_mask, 0);
    }

    #[test]
    fn unchanged_editor_round_trip_keeps_draft_applied() {
        functions::reset_active_functions();
        let mut app = AppState::new();
        app.active_tab = Tab::Equation;
        app.focus = Focus::Content;
        app.equation_page = EquationPage::FunctionDetail;
        app.function_detail_row = 1;
        let _ = app.handle_equation_event(event::EXE);
        assert_eq!(app.equation_page, EquationPage::ExpressionEditor);
        let _ = app.handle_equation_event(event::BACK);
        assert!(functions::with_active_functions(
            |set| set.slots[0].draft_matches_compiled
        ));
    }

    #[test]
    fn failed_function_compile_preserves_program_and_dirty_masks() {
        functions::reset_active_functions();
        let mut app = AppState::new();
        app.active_tab = Tab::Equation;
        app.focus = Focus::Content;
        app.equation_page = EquationPage::FunctionDetail;
        app.function_detail_row = 1;
        let before = functions::with_active_functions(|set| {
            set.slots[0].compiled.as_ref().unwrap().evaluate(0.5, 0.25)
        });
        let _ = app.handle_equation_event(event::EXE);
        app.editor.load_source(b"sin(");
        app.surface_dirty_mask = 0;
        app.intersection_dirty_mask = 0;
        app.dirty.surface = false;
        let _ = app.handle_equation_event(event::EXE);
        assert_eq!(app.equation_page, EquationPage::ExpressionEditor);
        assert_eq!(app.surface_dirty_mask, 0);
        assert_eq!(app.intersection_dirty_mask, 0);
        assert!(!app.dirty.surface);
        assert_eq!(
            functions::with_active_functions(|set| set.slots[0]
                .compiled
                .as_ref()
                .unwrap()
                .evaluate(0.5, 0.25)),
            before
        );
    }

    #[test]
    fn custom_color_back_discards_function_draft() {
        functions::reset_active_functions();
        let mut app = AppState::new();
        app.active_tab = Tab::Equation;
        app.focus = Focus::Content;
        app.equation_page = EquationPage::CustomColor;
        let original = functions::with_active_functions(|set| set.slots[0].custom_rgb);
        app.custom_color_draft = original;
        let _ = app.handle_equation_event(event::RIGHT);
        assert_ne!(app.custom_color_draft, original);
        let _ = app.handle_equation_event(event::BACK);
        assert_eq!(app.equation_page, EquationPage::FunctionDetail);
        assert_eq!(
            functions::with_active_functions(|set| set.slots[0].custom_rgb),
            original
        );
    }
}

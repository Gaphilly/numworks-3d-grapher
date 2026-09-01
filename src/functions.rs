//! Fixed-capacity state for four independently editable mathematical surfaces.
//!
//! A function's source draft is deliberately separate from its last compiled
//! postfix bytecode. Invalid or cancelled edits may be retained for later work,
//! but can never replace the expression currently feeding sampled geometry.

use crate::expression::{CompiledExpression, MAX_EXPRESSION_LENGTH};
use crate::graph::{Rgb888, SurfacePalette};

pub const MAX_FUNCTIONS: usize = 4;
pub const MAX_FUNCTION_PAIRS: usize = 6;
#[allow(dead_code)]
pub const ALL_FUNCTION_BITS: u8 = (1 << MAX_FUNCTIONS) - 1;
#[allow(dead_code)]
pub const ALL_PAIR_BITS: u8 = (1 << MAX_FUNCTION_PAIRS) - 1;

/// Deterministic pair order used by caches, UI rows, and visibility bits.
pub const FUNCTION_PAIRS: [(usize, usize); MAX_FUNCTION_PAIRS] =
    [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)];

/// One function's active program, independent draft, and appearance.
pub struct FunctionSlot {
    pub compiled: Option<CompiledExpression>,
    pub draft_source: [u8; MAX_EXPRESSION_LENGTH],
    pub draft_length: u8,
    pub draft_matches_compiled: bool,
    pub enabled: bool,
    pub palette: SurfacePalette,
    pub custom_rgb: Rgb888,
}

impl FunctionSlot {
    const fn empty(palette: SurfacePalette) -> Self {
        Self {
            compiled: None,
            draft_source: [0; MAX_EXPRESSION_LENGTH],
            draft_length: 0,
            draft_matches_compiled: true,
            enabled: false,
            palette,
            custom_rgb: Rgb888 {
                red: 0,
                green: 0,
                blue: 0,
            },
        }
    }

    pub fn draft(&self) -> &[u8] {
        &self.draft_source[..self.draft_length as usize]
    }

    pub fn can_enable(&self) -> bool {
        self.compiled.is_some()
    }

    pub fn set_draft(&mut self, source: &[u8]) {
        self.draft_source = [0; MAX_EXPRESSION_LENGTH];
        let length = core::cmp::min(source.len(), MAX_EXPRESSION_LENGTH);
        self.draft_source[..length].copy_from_slice(&source[..length]);
        self.draft_length = length as u8;
        self.draft_matches_compiled = false;
    }

    pub fn compile_draft(&mut self) -> Result<(), crate::expression::ParseError> {
        let source = core::str::from_utf8(self.draft()).unwrap_or("");
        let compiled = CompiledExpression::compile(source)?;
        self.compiled = Some(compiled);
        self.draft_matches_compiled = true;
        self.enabled = true;
        Ok(())
    }
}

/// Persistent bounded collection of every user-visible function.
pub struct FunctionSet {
    pub slots: [FunctionSlot; MAX_FUNCTIONS],
}

impl FunctionSet {
    const EMPTY: Self = Self {
        slots: [
            FunctionSlot::empty(SurfacePalette::Blue),
            FunctionSlot::empty(SurfacePalette::Blue),
            FunctionSlot::empty(SurfacePalette::Blue),
            FunctionSlot::empty(SurfacePalette::Blue),
        ],
    };

    pub fn initialize(&mut self) {
        if self.slots[0].compiled.is_some() {
            return;
        }
        let palettes = [
            SurfacePalette::Blue,
            SurfacePalette::Red,
            SurfacePalette::Green,
            SurfacePalette::Orange,
        ];
        let mut index = 0;
        while index < MAX_FUNCTIONS {
            self.slots[index].palette = palettes[index];
            self.slots[index].custom_rgb = Rgb888::DEFAULT_CUSTOM;
            index += 1;
        }
        self.slots[0].set_draft(b"sin(x) * cos(y)");
        if self.slots[0].compile_draft().is_err() {
            self.slots[0].enabled = false;
        }
    }

    pub fn enabled_mask(&self) -> u8 {
        let mut mask = 0;
        let mut index = 0;
        while index < MAX_FUNCTIONS {
            if self.slots[index].enabled && self.slots[index].compiled.is_some() {
                mask |= 1 << index;
            }
            index += 1;
        }
        mask
    }
}

static mut ACTIVE_FUNCTIONS: FunctionSet = FunctionSet::EMPTY;

/// Gives the cooperative application loop exclusive access to function state.
///
/// SAFETY: no interrupt handler touches this private static and callbacks never
/// re-enter this function. References do not escape the callback.
pub fn with_active_functions<R>(callback: impl FnOnce(&mut FunctionSet) -> R) -> R {
    #[cfg(test)]
    let _guard = TEST_FUNCTION_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    unsafe { callback(&mut *core::ptr::addr_of_mut!(ACTIVE_FUNCTIONS)) }
}

#[cfg(test)]
pub fn reset_active_functions() {
    with_active_functions(|functions| {
        *functions = FunctionSet::EMPTY;
        functions.initialize();
    });
}

#[cfg(test)]
static TEST_FUNCTION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub const fn pair_index(first: usize, second: usize) -> Option<usize> {
    let mut index = 0;
    while index < MAX_FUNCTION_PAIRS {
        let pair = FUNCTION_PAIRS[index];
        if pair.0 == first && pair.1 == second {
            return Some(index);
        }
        index += 1;
    }
    None
}

pub fn pair_mask_for_function(function: usize) -> u8 {
    let mut mask = 0;
    let mut index = 0;
    while index < MAX_FUNCTION_PAIRS {
        let pair = FUNCTION_PAIRS[index];
        if pair.0 == function || pair.1 == function {
            mask |= 1 << index;
        }
        index += 1;
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::function::SurfaceFunction;

    #[test]
    fn defaults_and_pair_order_are_stable() {
        let mut functions = FunctionSet::EMPTY;
        functions.initialize();
        assert_eq!(functions.enabled_mask(), 1);
        assert_eq!(functions.slots[0].draft(), b"sin(x) * cos(y)");
        assert!(functions.slots[0].compiled.is_some());
        assert!(functions.slots[0].draft_matches_compiled);
        assert_eq!(functions.slots[0].palette, SurfacePalette::Blue);
        assert_eq!(functions.slots[1].palette, SurfacePalette::Red);
        assert_eq!(functions.slots[2].palette, SurfacePalette::Green);
        assert_eq!(functions.slots[3].palette, SurfacePalette::Orange);
        for slot in &functions.slots[1..] {
            assert!(!slot.enabled);
            assert!(slot.compiled.is_none());
            assert!(slot.draft().is_empty());
        }
        assert_eq!(
            FUNCTION_PAIRS,
            [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]
        );
        assert_eq!(pair_mask_for_function(0), 0b000111);
        assert_eq!(pair_mask_for_function(3), 0b110100);
    }

    #[test]
    fn invalid_draft_preserves_compiled_expression() {
        let mut functions = FunctionSet::EMPTY;
        functions.initialize();
        let before = functions.slots[0]
            .compiled
            .as_ref()
            .unwrap()
            .evaluate(0.5, 0.25);
        functions.slots[0].set_draft(b"sin(");
        assert!(functions.slots[0].compile_draft().is_err());
        assert_eq!(
            functions.slots[0]
                .compiled
                .as_ref()
                .unwrap()
                .evaluate(0.5, 0.25),
            before
        );
        assert!(!functions.slots[0].draft_matches_compiled);
    }

    #[test]
    fn four_slots_keep_independent_sources_programs_and_colors() {
        let mut functions = FunctionSet::EMPTY;
        functions.initialize();
        let expressions: [&[u8]; 3] = [b"x", b"y", b"x+y"];
        let mut index = 1;
        while index < MAX_FUNCTIONS {
            functions.slots[index].set_draft(expressions[index - 1]);
            functions.slots[index].compile_draft().unwrap();
            index += 1;
        }
        assert_eq!(functions.enabled_mask(), ALL_FUNCTION_BITS);
        assert_eq!(
            functions.slots[1]
                .compiled
                .as_ref()
                .unwrap()
                .evaluate(2.0, 3.0),
            2.0
        );
        assert_eq!(
            functions.slots[2]
                .compiled
                .as_ref()
                .unwrap()
                .evaluate(2.0, 3.0),
            3.0
        );
        assert_eq!(
            functions.slots[3]
                .compiled
                .as_ref()
                .unwrap()
                .evaluate(2.0, 3.0),
            5.0
        );
        assert_eq!(core::mem::size_of::<FunctionSet>(), 2_480);
    }
}

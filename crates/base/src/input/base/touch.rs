//! Touch selection for the input engine: grab handles and the edit menu.
//!
//! A touch selection carries a handle at each end which the styled layer draws
//! and which drags through [`InputBaseState::begin_edge_drag`], plus the edit
//! menu the platform would show over it. A double tap makes one, selecting the
//! word under the finger, and so does Select All from the menu itself.
//!
//! GPUI also made one from a long press. WGPUI dispatches no long-press event
//! and `MouseEvent` is sealed, so that producer cannot be rebuilt here and the
//! gesture is absent rather than inert; `paint_mouse_listeners` in `element.rs`
//! records the same reasoning for the element side.
//!
//! The touch selection remembers the range it made. As soon as the selection
//! is something else — the caret moved, text was typed, another cursor was
//! added — the handles and the menu are gone, without anyone having to hide
//! them. Only the touch selection and the menu's own actions move the range and
//! carry the touch selection along.

use gpui::{Context, Pixels, Point, TouchPhase, Window, point};

use super::{InputBaseState, InputModeKind};
use crate::touch_selection::{
    EdgeDrag, SelectionEdge, TouchSelectionSnapshot, caret_in_view, caret_line_box,
};

/// The selection a touch gesture made, and what it is doing now.
#[derive(Debug, Default)]
pub(super) struct TouchSelection {
    /// The range the gesture left; `None` when no touch selection is live.
    range: Option<(usize, usize)>,
    menu_open: bool,
    drag: Option<EdgeDrag>,
}

impl<M: InputModeKind> InputBaseState<M> {
    /// The live touch selection, laid out for the handles and the edit menu.
    ///
    /// `None` when the last touch selection has since changed into something
    /// else, or when its ends are not laid out (scrolled out of view).
    pub fn touch_selection(&self) -> Option<TouchSelectionSnapshot> {
        let range = self.touch_selection.range?;
        let selection = self.active_selection();
        if (selection.start, selection.end) != range {
            return None;
        }

        let layout = self.last_layout.as_ref()?;
        let line_height = layout.line_height;
        let laid_out = layout.visible_range_offset.clone();
        let origin = self.last_bounds?.origin;
        let viewport = self.input_bounds;
        // An end scrolled out of the input gets no handle. Its line may not
        // even be laid out; it then stands just outside the viewport on its
        // side, which is all the other end's drag needs to know about it.
        let caret_box = |offset: usize, stand_in_y: Pixels| {
            let (_, _, position) = self.line_and_position_for_offset(offset);
            match position.filter(|_| laid_out.contains(&offset) || laid_out.end == offset) {
                Some(position) => caret_line_box(origin + position, line_height),
                None => caret_line_box(point(viewport.left(), stand_in_y), line_height),
            }
        };
        let start = caret_box(range.0, viewport.top() - line_height);
        let end = caret_box(range.1, viewport.bottom());
        Some(
            TouchSelectionSnapshot::new(start, end)
                .with_edge_visible(SelectionEdge::Start, caret_in_view(start, viewport))
                .with_edge_visible(SelectionEdge::End, caret_in_view(end, viewport))
                .with_menu_open(self.touch_selection.menu_open)
                .with_dragging(self.touch_selection.drag.map(|drag| drag.edge())),
        )
    }

    /// Records the current selection as the one the touch gesture made.
    fn retain_touch_selection(&mut self) {
        let selection = self.active_selection();
        self.touch_selection.range = Some((selection.start, selection.end));
    }

    /// Keeps the current selection as a touch selection, with the menu open
    /// over it: what a double tap's word selection asks for.
    pub(super) fn keep_touch_selection(&mut self, cx: &mut Context<Self>) {
        self.retain_touch_selection();
        self.touch_selection.menu_open = true;
        self.touch_selection.drag = None;
        cx.notify();
    }

    /// Drops the handles and the edit menu. Called where the selection is
    /// about to be moved by something other than the gesture.
    pub(super) fn dismiss_touch_selection(&mut self, cx: &mut Context<Self>) {
        if self.touch_selection.range.is_none() {
            return;
        }
        self.touch_selection = TouchSelection::default();
        cx.notify();
    }

    /// Closes the edit menu and keeps the selection with its handles.
    ///
    /// The menu's own actions end here: Copy has done its work, Select All
    /// reopens the menu over the new range through [`Self::select_all`].
    pub fn close_edit_menu(&mut self, cx: &mut Context<Self>) {
        if !self.touch_selection.menu_open {
            return;
        }
        self.touch_selection.menu_open = false;
        cx.notify();
    }

    /// Reopens the edit menu when `position` is on the touch selection, the
    /// way a tap on selected text asks for it. Returns whether it did.
    pub(super) fn reopen_edit_menu_at(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(snapshot) = self.touch_selection() else {
            return false;
        };
        if snapshot.is_empty() {
            return false;
        }
        let (offset, _, _) = self.resolve_mouse_position(position);
        let selection = self.active_selection();
        if offset <= selection.start || offset >= selection.end {
            return false;
        }
        self.touch_selection.menu_open = true;
        cx.notify();
        true
    }

    /// Steps the menu aside while the content scrolls under a finger, and
    /// brings it back over the handles once the finger lifts.
    pub(super) fn edit_menu_on_scroll(&mut self, phase: TouchPhase, cx: &mut Context<Self>) {
        if self.touch_selection.range.is_none() {
            return;
        }
        match phase {
            // WGPUI folds a cancelled touch into `Ended` — `TouchPhase` has no
            // `Cancelled` variant — so this arm covers both.
            TouchPhase::Ended => {
                if !self.touch_selection.menu_open {
                    self.touch_selection.menu_open = true;
                    cx.notify();
                }
            }
            _ => self.close_edit_menu(cx),
        }
    }

    /// Selects everything and, when a touch selection is live, keeps the
    /// handles and the edit menu over the new range.
    pub fn select_all_from_edit_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let touch = self.touch_selection.range.is_some();
        self.select_all(window, cx);
        if touch {
            self.retain_touch_selection();
            self.touch_selection.menu_open = true;
            cx.notify();
        }
    }

    /// Starts dragging one end of the touch selection from `finger`.
    ///
    /// The other end stays where it is. The menu closes for the duration of the
    /// drag and reopens on [`Self::end_edge_drag`].
    pub fn begin_edge_drag(
        &mut self,
        edge: SelectionEdge,
        finger: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.touch_selection() else {
            return;
        };
        self.undo_manager.break_transaction_coalescing();
        self.selected_word_range = None;
        self.active_selection_mut().reversed = edge == SelectionEdge::Start;
        self.touch_selection.drag = Some(EdgeDrag::begin(edge, snapshot.edge(edge), finger));
        self.touch_selection.menu_open = false;
        cx.notify();
    }

    /// Moves the dragged end to the text under `finger`.
    ///
    /// Past the top or bottom of a multi-line input the content scrolls under
    /// the finger, so the end can reach text that was out of view.
    pub fn update_edge_drag(&mut self, finger: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.touch_selection.drag else {
            return;
        };
        let position = drag.text_position(finger);
        self.extend_edge_drag_to(position, cx);

        if self.is_single_line() {
            return;
        }
        self.auto_scroll.last_drag_position = Some(position);
        let delta = crate::AutoScroll::compute_delta(position.y, self.input_bounds);
        // Input's ScrollHandle uses negative-y-is-down; negate the positive-towards-bottom delta.
        let scroll_delta = delta.map(|delta| -delta);
        self.auto_scroll.set(scroll_delta, cx, |delta, state, cx| {
            let current = state.scroll_handle.offset();
            state.update_scroll_offset(Some(point(current.x, current.y + delta)), cx);
            if let Some(position) = state.auto_scroll.last_drag_position {
                state.extend_edge_drag_to(position, cx);
            }
        });
    }

    fn extend_edge_drag_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.touch_selection.drag.is_none() {
            return;
        }
        let (offset, line_end_affinity, _) = self.resolve_mouse_position(position);
        let before = *self.active_selection();
        self.select_to_with_affinity(offset, line_end_affinity, cx);
        // A handle never collapses the selection: at the other end it stops,
        // and the finger has to pass that end to swap the two.
        if self.active_selection().is_empty() {
            *self.active_selection_mut() = before;
            return;
        }
        // Dragging one end past the other swaps them: the selection now runs
        // the other way and the finger holds what became the other handle.
        let edge = if self.active_selection().reversed {
            SelectionEdge::Start
        } else {
            SelectionEdge::End
        };
        if let Some(drag) = self.touch_selection.drag.as_mut() {
            drag.set_edge(edge);
        }
        self.retain_touch_selection();
        cx.notify();
    }

    /// Ends the handle drag and reopens the edit menu.
    pub fn end_edge_drag(&mut self, cx: &mut Context<Self>) {
        if self.touch_selection.drag.take().is_none() {
            return;
        }
        self.auto_scroll.stop();
        if self.active_selection().is_empty() {
            self.active_selection_mut().reversed = false;
        }
        self.touch_selection.menu_open = true;
        cx.notify();
    }

    #[cfg(test)]
    pub(super) fn is_edit_menu_open(&self) -> bool {
        self.touch_selection.menu_open
    }

    #[cfg(test)]
    pub(super) fn touch_selection_range(&self) -> Option<std::ops::Range<usize>> {
        self.touch_selection.range.map(|(start, end)| start..end)
    }
}

#[cfg(test)]
mod tests {
    use gpui::{
        AppContext as _, Context, Entity, IntoElement, MouseButton, ParentElement as _, Render,
        Styled as _, TestAppContext, VisualTestContext, Window, div, point, px,
    };

    use crate::input::{InputState, TextareaState};
    use crate::touch_selection::SelectionEdge;

    struct TouchRoot(Entity<InputState>);

    impl Render for TouchRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.0.clone())
        }
    }

    struct TextareaRoot(Entity<TextareaState>);

    impl Render for TextareaRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            // Short enough that forty lines scroll.
            div().w(px(300.)).h(px(80.)).child(self.0.clone())
        }
    }

    fn open_input<'a>(
        cx: &'a mut TestAppContext,
        value: &str,
    ) -> (Entity<InputState>, &'a mut VisualTestContext) {
        cx.update(crate::init);
        let value = value.to_string();
        let (root, cx) = cx.add_window_view(move |window, cx| {
            let input = cx.new(|cx| {
                let mut state = InputState::new(window, cx);
                state.set_value(value, window, cx);
                state
            });
            TouchRoot(input)
        });
        let input = root.read_with(cx, |root, _| root.0.clone());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        (input, cx)
    }

    /// Double taps at `at`, which selects the word under it and makes a touch
    /// selection. A tap arrives as mouse events, so the touch that began it is
    /// noted first: that is what tells the input they came from a finger.
    ///
    /// WGPUI dispatches no long-press event, so this is the only gesture that
    /// makes a touch selection; it is the double-click-and-drag GPUI's long
    /// press stood in for.
    fn double_tap(cx: &mut VisualTestContext, at: (f32, f32)) {
        let position = point(px(at.0), px(at.1));
        cx.update(|_, cx| crate::GlobalState::note_touch(cx));
        for click_count in [1, 2] {
            cx.simulate_event(gpui::MouseDownEvent {
                position,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count,
                first_mouse: false,
            });
            cx.simulate_event(gpui::MouseUpEvent {
                position,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count,
            });
        }
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }

    /// The window position of the caret before `offset`, in the input's line.
    fn caret_at(input: &Entity<InputState>, cx: &VisualTestContext, offset: usize) -> (f32, f32) {
        input.read_with(cx, |state, _| {
            let (_, _, position) = state.line_and_position_for_offset(offset);
            let position = state.last_bounds.unwrap().origin + position.unwrap();
            let line_height = state.last_layout.as_ref().unwrap().line_height;
            (position.x.into(), (position.y + line_height * 0.5).into())
        })
    }

    #[gpui::test]
    fn double_tap_selects_word_with_handles_and_menu(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx, "quick select value");
        let at = caret_at(&input, cx, 8);
        double_tap(cx, at);
        input.read_with(cx, |state, _| {
            assert_eq!(state.selected_text().to_string(), "select");
            let snapshot = state
                .touch_selection()
                .expect("a double tap is a touch selection");
            assert!(!snapshot.is_empty());
            assert!(snapshot.is_menu_open());
            assert!(snapshot.start().left() < snapshot.end().left());
        });
    }

    #[gpui::test]
    fn double_tap_on_empty_input_places_caret_with_menu(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx, "");
        let caret = caret_at(&input, cx, 0);
        double_tap(cx, caret);
        input.read_with(cx, |state, _| {
            assert_eq!(state.selected_range(), 0..0);
            let snapshot = state.touch_selection().expect("caret still gets a menu");
            assert!(snapshot.is_empty());
            assert!(snapshot.is_menu_open());
        });
    }

    #[gpui::test]
    fn dragging_a_handle_moves_that_end_only(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx, "quick select value");
        let start = caret_at(&input, cx, 8);
        double_tap(cx, start);

        // The finger holds the end knob, which hangs below the line.
        let end_caret = caret_at(&input, cx, 12);
        let finger = (end_caret.0, end_caret.1 + 20.);
        cx.update(|_, cx| {
            input.update(cx, |state, cx| {
                state.begin_edge_drag(SelectionEdge::End, point(px(finger.0), px(finger.1)), cx);
            });
        });
        input.read_with(cx, |state, _| {
            let snapshot = state.touch_selection().unwrap();
            assert_eq!(snapshot.dragging(), Some(SelectionEdge::End));
            assert!(!snapshot.is_menu_open());
        });

        let target = caret_at(&input, cx, 18);
        cx.update(|_, cx| {
            input.update(cx, |state, cx| {
                state.update_edge_drag(point(px(target.0), px(target.1 + 20.)), cx);
            });
        });
        input.read_with(cx, |state, _| {
            assert_eq!(state.selected_text().to_string(), "select value");
        });

        // Pull the start handle past the end: the ends swap, the finger keeps
        // its handle.
        cx.update(|_, cx| {
            input.update(cx, |state, cx| {
                state.end_edge_drag(cx);
                assert!(state.is_edit_menu_open());
                let start_caret = state.touch_selection().unwrap().start();
                state.begin_edge_drag(SelectionEdge::Start, start_caret.origin, cx);
            });
        });
        let target = caret_at(&input, cx, 0);
        cx.update(|_, cx| {
            input.update(cx, |state, cx| {
                state.update_edge_drag(point(px(target.0), px(target.1)), cx);
            });
        });
        input.read_with(cx, |state, _| {
            assert_eq!(state.selected_text().to_string(), "quick select value");
            assert_eq!(state.touch_selection_range(), Some(0..18));
        });
    }

    #[gpui::test]
    fn dragging_one_handle_past_the_other_swaps_them(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx, "quick select value");
        let at = caret_at(&input, cx, 8);
        double_tap(cx, at);
        input.read_with(cx, |state, _| {
            assert_eq!(state.selected_text().to_string(), "select");
        });

        // Take the start handle and pull it past the end of "select" to the
        // end of the text: the finger now holds the end handle, and the
        // selection runs from the old end forward.
        let start_caret = caret_at(&input, cx, 6);
        cx.update(|_, cx| {
            input.update(cx, |state, cx| {
                state.begin_edge_drag(
                    SelectionEdge::Start,
                    point(px(start_caret.0), px(start_caret.1)),
                    cx,
                );
            });
        });
        let target = caret_at(&input, cx, 18);
        cx.update(|_, cx| {
            input.update(cx, |state, cx| {
                state.update_edge_drag(point(px(target.0), px(target.1)), cx);
            });
        });
        input.read_with(cx, |state, _| {
            assert_eq!(state.selected_text().to_string(), " value");
            assert_eq!(
                state.touch_selection().unwrap().dragging(),
                Some(SelectionEdge::End)
            );
        });

        // And back across again: it is the start handle once more.
        let target = caret_at(&input, cx, 0);
        cx.update(|_, cx| {
            input.update(cx, |state, cx| {
                state.update_edge_drag(point(px(target.0), px(target.1)), cx);
                state.end_edge_drag(cx);
            });
        });
        input.read_with(cx, |state, _| {
            assert_eq!(state.selected_text().to_string(), "quick select");
            assert!(state.is_edit_menu_open());
        });
    }

    #[gpui::test]
    fn touch_selection_goes_away_when_something_else_moves_the_selection(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx, "quick select value");
        let start = caret_at(&input, cx, 2);
        double_tap(cx, start);
        input.read_with(cx, |state, _| assert!(state.touch_selection().is_some()));

        // Select All from the menu keeps it, over the new range.
        cx.update(|window, cx| {
            input.update(cx, |state, cx| state.select_all_from_edit_menu(window, cx));
        });
        input.read_with(cx, |state, _| {
            assert_eq!(state.selected_range(), 0..18);
            assert!(state.touch_selection().is_some());
            assert!(state.is_edit_menu_open());
        });

        // Copy closes the menu but keeps the handles.
        cx.update(|_, cx| input.update(cx, |state, cx| state.close_edit_menu(cx)));
        input.read_with(cx, |state, _| {
            let snapshot = state.touch_selection().unwrap();
            assert!(!snapshot.is_menu_open());
        });

        // Typing replaces the selection; nothing is left to hold a handle.
        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.replace_text_in_range_silent(None, "x", window, cx);
            });
        });
        input.read_with(cx, |state, _| assert!(state.touch_selection().is_none()));

        // A press elsewhere drops it too.
        double_tap(cx, start);
        input.read_with(cx, |state, _| assert!(state.touch_selection().is_some()));
        let elsewhere = caret_at(&input, cx, 0);
        cx.simulate_mouse_down(
            point(px(elsewhere.0), px(elsewhere.1)),
            MouseButton::Left,
            Default::default(),
        );
        input.read_with(cx, |state, _| assert!(state.touch_selection().is_none()));
    }

    #[gpui::test]
    fn scrolling_closes_the_menu_and_keeps_the_handles(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (root, cx) = cx.add_window_view(|window, cx| {
            let textarea = cx.new(|cx| {
                let mut state = TextareaState::new(window, cx);
                let text = (0..40).map(|ix| format!("line {ix}")).collect::<Vec<_>>();
                state.set_value(text.join("\n"), window, cx);
                state
            });
            TextareaRoot(textarea)
        });
        let textarea = root.read_with(cx, |root, _| root.0.clone());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let start = textarea.read_with(cx, |state, _| {
            let (_, _, position) = state.line_and_position_for_offset(2);
            let position = state.last_bounds.unwrap().origin + position.unwrap();
            (f32::from(position.x), f32::from(position.y) + 4.)
        });
        double_tap(cx, start);
        textarea.read_with(cx, |state, _| {
            assert_eq!(state.selected_text().to_string(), "line");
            assert!(state.touch_selection().unwrap().is_menu_open());
        });

        cx.update(|_, cx| {
            textarea.update(cx, |state, cx| {
                state.set_scroll_offset(point(px(0.), px(-8.)), cx);
                state.close_edit_menu(cx);
            });
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        textarea.read_with(cx, |state, _| {
            let snapshot = state.touch_selection().expect("handles follow the text");
            assert!(!snapshot.is_menu_open());
            assert_eq!(state.selected_text().to_string(), "line");
        });

        // Scrolled far enough, the selection leaves the viewport: no handle
        // for it, and no menu anchored to nothing.
        cx.update(|_, cx| {
            textarea.update(cx, |state, cx| {
                state.set_scroll_offset(point(px(0.), px(-600.)), cx);
            });
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        textarea.read_with(cx, |state, _| {
            let snapshot = state
                .touch_selection()
                .expect("the selection is still the touch one");
            assert!(!snapshot.is_edge_visible(SelectionEdge::Start));
            assert!(!snapshot.is_edge_visible(SelectionEdge::End));
            assert_eq!(snapshot.bounds(), None);
        });
    }
}

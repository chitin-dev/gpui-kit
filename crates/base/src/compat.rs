//! The GPUI vocabulary this fork is written against that WGPUI does not carry.
//!
//! WGPUI (`gpui-ce`) is the same generation of GPUI as the `gpui-pre` snapshot
//! this fork tracks; what it drops is a set of layers that gpui-kit's code
//! names directly. Three kinds of thing live here, and they are not alike:
//!
//! - **Semantic vocabulary.** [`Role`], [`Toggled`], [`Orientation`] and
//!   [`AccessibleAction`] are the words gpui-kit's own public API is expressed
//!   in: `Button::role(Role::Button)` is Base's API, not GPUI's. WGPUI depends
//!   on no `accesskit`, publishes no accessibility tree and has no
//!   `Element::write_a11y_info` hook, so there is no tree to publish these into
//!   — but the vocabulary is still how a caller states intent, so it is kept
//!   and the plumbing that consumed it is gone. Values here are inert, and
//!   [`A11yElementExt`] is where they are inert in the code: the builders GPUI
//!   had on `Stateful` take their value and hand the element back.
//! - **Behavior WGPUI never had.** [`OngoingScroll`] (axis-locked gestures),
//!   [`Anchor`] (corner placement, with [`BoundsExt`]) and the spring physics
//!   ([`SpringConfig`], [`SpringState`], [`SpringTarget`]) are real
//!   computation this fork needs and WGPUI does not supply. These are
//!   implemented here rather than stubbed, as are [`HitboxExt`],
//!   [`FlexExt`] and [`paint_shaped_line`].
//! - **A flag GPUI used to own.** [`ReduceMotionExt`] supplies the
//!   `App::reduce_motion` pair that `reduce_motion` writes into; WGPUI has no
//!   such flag, and Base's own platform readers need somewhere to put their
//!   answer.
//!
//! What is deliberately *not* here is touch gestures. WGPUI reports a finger
//! only as a [`gpui::ScrollWheelEvent`] carrying a `TouchPhase`; it dispatches
//! no `TouchDragEvent` and no `LongPressEvent`, and `gpui::MouseEvent` is
//! sealed, so a listener for either cannot be written from outside GPUI. The
//! gesture layers built on them are absent rather than inert, because a
//! listener that can never be constructed is not something a shim can stand in
//! for.

use std::{ops::Range, sync::Arc, sync::OnceLock, time::Instant};

use gpui::{
    Animation, App, Axis, Bounds, Canvas, Corner, Global, Hitbox, Hsla, ListOffset, ListState,
    Pixels, Point, Result, ShapedLine, SharedString, Size, Styled, TextAlign, TextLayout,
    TouchPhase, TransformationMatrix, Window, WrappedLineLayout, canvas, px,
};

// ── Semantic vocabulary ─────────────────────────────────────────────────────

/// What a thing is, for a client that would be told about it.
///
/// The variants are the AccessKit role names, so a role read here means what it
/// means anywhere else; WGPUI publishes none of them. `Role::Unknown` is the
/// default so a caller that has no better answer can say so rather than pick a
/// wrong one.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Role {
    /// No role is known.
    #[default]
    Unknown,
    /// A clickable thing that performs an action.
    Button,
    /// A two-state check box.
    CheckBox,
    /// A radio button in a group.
    RadioButton,
    /// An on/off switch.
    Switch,
    /// A single-line text field.
    TextInput,
    /// A multi-line text field.
    MultilineTextInput,
    /// A text field that takes a password.
    PasswordInput,
    /// A text field that takes an email address.
    EmailInput,
    /// A text field that takes a telephone number.
    PhoneNumberInput,
    /// A text field that takes a URL.
    UrlInput,
    /// A text field that takes a date.
    DateInput,
    /// A text field that takes a time.
    TimeInput,
    /// A text field that takes a date and a time.
    DateTimeInput,
    /// A numeric field stepped by a control.
    SpinButton,
    /// A field holding a number chosen along a track.
    Slider,
    /// A control that opens a list of choices.
    ComboBox,
    /// A list of choices.
    ListBox,
    /// One choice in a [`Role::ListBox`].
    ListBoxOption,
    /// A menu.
    Menu,
    /// The bar holding a window's menus.
    MenuBar,
    /// One entry in a menu.
    MenuItem,
    /// A group of radio buttons.
    RadioGroup,
    /// A heading.
    Heading,
    /// A hyperlink.
    Link,
    /// An image or drawing.
    Image,
    /// A table.
    Table,
    /// A row group in a table.
    RowGroup,
    /// A row in a table.
    Row,
    /// A cell in a table.
    Cell,
    /// A column header in a table.
    ColumnHeader,
    /// A tree.
    Tree,
    /// One node in a tree.
    TreeItem,
    /// A list.
    List,
    /// One entry in a list.
    ListItem,
    /// A toolbar.
    Toolbar,
    /// A tab strip.
    TabList,
    /// One tab.
    Tab,
    /// A page or dialog area.
    Region,
    /// A group of related things.
    Group,
    /// A form.
    Form,
    /// A dialog.
    Dialog,
    /// A dialog that interrupts to report something.
    AlertDialog,
    /// A message that reports something without interrupting.
    Alert,
    /// A status line.
    Status,
    /// A progress indicator.
    ProgressIndicator,
    /// A log.
    Log,
    /// A navigation landmark.
    Navigation,
    /// A tooltip.
    Tooltip,
    /// A structure whose only purpose is to hold children.
    GenericContainer,
}

/// Whether a two-state thing is on, off, or neither.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Toggled {
    /// Off.
    False,
    /// On.
    True,
    /// Neither, as a tri-state check box reports.
    Mixed,
}

/// Which way an ordered thing runs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Orientation {
    /// Left to right.
    Horizontal,
    /// Top to bottom.
    Vertical,
}

/// An action an accessibility client can ask a thing to perform.
///
/// Base's controls name the action they would answer in their own API. WGPUI
/// delivers no such request, so nothing raises one.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AccessibleAction {
    /// Press the thing.
    Click,
    /// Move focus to the thing.
    Focus,
    /// Take focus away from the thing.
    Blur,
    /// Close the thing.
    Collapse,
    /// Open the thing.
    Expand,
    /// Raise the thing's value by one step.
    Increment,
    /// Lower the thing's value by one step.
    Decrement,
    /// Set the thing's value outright.
    SetValue,
}

// ── Accessibility builders ──────────────────────────────────────────────────

/// The accessibility builders GPUI's `StatefulInteractiveElement` carried.
///
/// Every method takes its value and hands `self` back untouched. A caller that
/// writes `.role(Role::Button).aria_label("Save")` is describing what the
/// element *is*, and that description is still worth writing down: it is how
/// the component's intent is recorded at the place the intent lives. WGPUI
/// publishes no tree for it to reach, so nothing downstream reads it, and the
/// builders are where the two facts meet without pretending otherwise.
///
/// This is a trait rather than an inherent method set because WGPUI's
/// `Stateful` is GPUI's own type; an extension trait is the only place a fork
/// can put a method on it. Inherent methods on gpui-kit's own types still win
/// where they exist, so `Button::role` keeps its meaning.
pub trait A11yElementExt: Sized {
    /// States what the element is.
    fn role(self, role: impl Into<crate::RoleOverride>) -> Self;

    /// States the developer-assigned name a client could reach this element by.
    fn accessibility_id(self, id: impl Into<gpui::SharedString>) -> Self;

    /// States the element's name.
    fn aria_label(self, label: impl Into<gpui::SharedString>) -> Self;

    /// States the element's placeholder text, for a field that is empty.
    fn aria_placeholder(self, placeholder: impl Into<gpui::SharedString>) -> Self;

    /// States the element's longer description.
    fn aria_description(self, description: impl Into<gpui::SharedString>) -> Self;

    /// States whether the element is on, off, or neither.
    fn aria_toggled(self, toggled: Toggled) -> Self;

    /// States whether the element is open.
    fn aria_expanded(self, expanded: bool) -> Self;

    /// States whether the element is the selected one.
    fn aria_selected(self, selected: bool) -> Self;

    /// States which way the element's contents run.
    fn aria_orientation(self, orientation: Orientation) -> Self;

    /// States the element's value, for something numeric.
    fn aria_numeric_value(self, value: f64) -> Self;

    /// States the element's value in words.
    fn aria_value(self, value: impl Into<gpui::SharedString>) -> Self;

    /// States the lowest value the element accepts.
    fn aria_min_numeric_value(self, value: f64) -> Self;

    /// States the highest value the element accepts.
    fn aria_max_numeric_value(self, value: f64) -> Self;

    /// States the increment the element moves by.
    fn aria_numeric_value_step(self, step: f64) -> Self;

    /// States which position the element holds in its set.
    fn aria_position_in_set(self, position: usize) -> Self;

    /// States how many things the element's set holds.
    fn aria_size_of_set(self, size: usize) -> Self;

    /// States which row of a table the element is.
    fn aria_row_index(self, index: usize) -> Self;

    /// States how many rows the element's table has.
    fn aria_row_count(self, count: usize) -> Self;

    /// States which column of a table the element is.
    fn aria_column_index(self, index: usize) -> Self;

    /// States how many columns the element's table has.
    fn aria_column_count(self, count: usize) -> Self;

    /// States how deep the element sits in its hierarchy.
    fn aria_level(self, level: usize) -> Self;

    /// Registers what to do when a client asks for `action`.
    ///
    /// The handler is dropped. Nothing raises such a request here, and holding
    /// it would keep the entities it captures alive for no reader.
    fn on_a11y_action(
        self,
        action: AccessibleAction,
        handler: impl Fn(&AccessibleAction, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Self;
}

impl<T: gpui::InteractiveElement> A11yElementExt for T {
    fn role(self, _: impl Into<crate::RoleOverride>) -> Self {
        self
    }

    fn accessibility_id(self, _: impl Into<gpui::SharedString>) -> Self {
        self
    }

    fn aria_label(self, _: impl Into<gpui::SharedString>) -> Self {
        self
    }

    fn aria_placeholder(self, _: impl Into<gpui::SharedString>) -> Self {
        self
    }

    fn aria_description(self, _: impl Into<gpui::SharedString>) -> Self {
        self
    }

    fn aria_toggled(self, _: Toggled) -> Self {
        self
    }

    fn aria_expanded(self, _: bool) -> Self {
        self
    }

    fn aria_selected(self, _: bool) -> Self {
        self
    }

    fn aria_orientation(self, _: Orientation) -> Self {
        self
    }

    fn aria_numeric_value(self, _: f64) -> Self {
        self
    }

    fn aria_value(self, _: impl Into<gpui::SharedString>) -> Self {
        self
    }

    fn aria_min_numeric_value(self, _: f64) -> Self {
        self
    }

    fn aria_max_numeric_value(self, _: f64) -> Self {
        self
    }

    fn aria_numeric_value_step(self, _: f64) -> Self {
        self
    }

    fn aria_position_in_set(self, _: usize) -> Self {
        self
    }

    fn aria_size_of_set(self, _: usize) -> Self {
        self
    }

    fn aria_row_index(self, _: usize) -> Self {
        self
    }

    fn aria_row_count(self, _: usize) -> Self {
        self
    }

    fn aria_column_index(self, _: usize) -> Self {
        self
    }

    fn aria_column_count(self, _: usize) -> Self {
        self
    }

    fn aria_level(self, _: usize) -> Self {
        self
    }

    fn on_a11y_action(
        self,
        _: AccessibleAction,
        _: impl Fn(&AccessibleAction, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Self {
        self
    }
}

// ── Placement ───────────────────────────────────────────────────────────────

/// Which corner of a popup is placed at a point.
///
/// The name is the corner of the *popup*, so `Anchor::TopLeft` puts the popup's
/// top-left corner where it was asked to. [`Anchor::bounds_at`] is the whole of
/// the type's arithmetic; everything else about placement — flipping at the
/// viewport edge, clamping — is [`crate::Positioner`]'s.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Anchor {
    /// The popup's top-left corner.
    #[default]
    TopLeft,
    /// The popup's top edge, centered.
    TopCenter,
    /// The popup's top-right corner.
    TopRight,
    /// The popup's left edge, centered.
    LeftCenter,
    /// The popup's center.
    Center,
    /// The popup's right edge, centered.
    RightCenter,
    /// The popup's bottom-left corner.
    BottomLeft,
    /// The popup's bottom edge, centered.
    BottomCenter,
    /// The popup's bottom-right corner.
    BottomRight,
}

impl Anchor {
    /// The bounds of a `size`-shaped popup whose `self` corner sits at
    /// `position`.
    ///
    /// `Center` centers the popup on the point in both axes, which is what makes
    /// it different from the four edge variants rather than a fourth corner.
    pub fn bounds_at(self, position: Point<Pixels>, size: Size<Pixels>) -> Bounds<Pixels> {
        let origin = match self {
            Self::TopLeft => position,
            Self::TopCenter => position - Point::new(size.width / 2., px(0.)),
            Self::TopRight => position - Point::new(size.width, px(0.)),
            Self::LeftCenter => position - Point::new(px(0.), size.height / 2.),
            Self::Center => position - Point::new(size.width / 2., size.height / 2.),
            Self::RightCenter => position - Point::new(size.width, size.height / 2.),
            Self::BottomLeft => position - Point::new(px(0.), size.height),
            Self::BottomCenter => position - Point::new(size.width / 2., size.height),
            Self::BottomRight => position - Point::new(size.width, size.height),
        };
        Bounds::new(origin, size)
    }

    /// The same anchor on the other side of the popup along `axis`.
    ///
    /// A popup with no room on the side it asked for is flipped to the other,
    /// and only the axis that overflowed moves: a submenu anchored at its
    /// trigger's top-left falls to the bottom-left when the window ends below
    /// it, but stays on the left even if the window also ends to the right.
    /// An anchor already centred along that axis names no side, so it is
    /// unchanged — the flip is its own inverse either way.
    pub fn other_side_along(self, axis: Axis) -> Self {
        match axis {
            Axis::Horizontal => match self {
                Self::TopLeft => Self::TopRight,
                Self::LeftCenter => Self::RightCenter,
                Self::BottomLeft => Self::BottomRight,
                Self::TopRight => Self::TopLeft,
                Self::RightCenter => Self::LeftCenter,
                Self::BottomRight => Self::BottomLeft,
                Self::TopCenter | Self::Center | Self::BottomCenter => self,
            },
            Axis::Vertical => match self {
                Self::TopLeft => Self::BottomLeft,
                Self::TopCenter => Self::BottomCenter,
                Self::TopRight => Self::BottomRight,
                Self::BottomLeft => Self::TopLeft,
                Self::BottomCenter => Self::TopCenter,
                Self::BottomRight => Self::TopRight,
                Self::LeftCenter | Self::Center | Self::RightCenter => self,
            },
        }
    }

    /// The [`Corner`] this anchor sits on, for placement that knows only four.
    ///
    /// WGPUI's `Anchored` names a corner rather than an anchor, and an anchor
    /// on an edge midpoint or at the center names no corner at all. Those fall
    /// back to the nearest corner on the side they are already on, so a popup
    /// placed by corner still lands in the quadrant the anchor asked for.
    pub fn to_corner(self) -> Corner {
        match self {
            Self::TopLeft | Self::TopCenter | Self::LeftCenter | Self::Center => Corner::TopLeft,
            Self::TopRight | Self::RightCenter => Corner::TopRight,
            Self::BottomLeft | Self::BottomCenter => Corner::BottomLeft,
            Self::BottomRight => Corner::BottomRight,
        }
    }
}

/// The edge midpoints of a [`Bounds`].
///
/// GPUI names the four corners but only two of the four edge midpoints, and
/// popup placement needs all of them to sit a popup against any side of its
/// trigger.
pub trait BoundsExt {
    /// The bounds of a `size`-shaped rectangle whose `anchor` corner sits at
    /// `position`, the placement GPUI used to spell
    /// `Bounds::from_anchor_and_size`.
    ///
    /// It lives on this trait rather than on [`Anchor`] so that a call site
    /// keeps the shape it had: `Type::function(..)` resolves a trait's
    /// associated function as long as the trait is in scope, so
    /// `Bounds::from_anchor_and_size(..)` still names the type being built.
    fn from_anchor_and_size(
        anchor: Anchor,
        position: Point<Pixels>,
        size: Size<Pixels>,
    ) -> Bounds<Pixels>;

    /// The midpoint of the top edge.
    fn top_center(&self) -> Point<Pixels>;

    /// The midpoint of the bottom edge.
    fn bottom_center(&self) -> Point<Pixels>;

    /// The midpoint of the left edge.
    fn left_center(&self) -> Point<Pixels>;

    /// The midpoint of the right edge.
    fn right_center(&self) -> Point<Pixels>;
}

impl BoundsExt for Bounds<Pixels> {
    fn from_anchor_and_size(
        anchor: Anchor,
        position: Point<Pixels>,
        size: Size<Pixels>,
    ) -> Bounds<Pixels> {
        anchor.bounds_at(position, size)
    }

    fn top_center(&self) -> Point<Pixels> {
        Point::new(self.center().x, self.top())
    }

    fn bottom_center(&self) -> Point<Pixels> {
        Point::new(self.center().x, self.bottom())
    }

    fn left_center(&self) -> Point<Pixels> {
        Point::new(self.left(), self.center().y)
    }

    fn right_center(&self) -> Point<Pixels> {
        Point::new(self.right(), self.center().y)
    }
}

/// Whether a hitbox would be hit at a given position.
///
/// WGPUI answers that question only for the pointer's current position, and
/// keeps the positional hit test to itself. A caller holding an event knows the
/// position that event carried, so this asks the window's hit test — which
/// applies occlusion and content masks — and then confirms the position is
/// inside the hitbox. A hitbox the pointer is over but that the event's
/// position is outside of answers `false`, which is the safe direction: a
/// press outside a control must not move it.
pub trait HitboxExt {
    /// Whether a hit test at `position` would reach this hitbox.
    fn is_hovered_at(&self, position: Point<Pixels>, window: &Window) -> bool;
}

impl HitboxExt for Hitbox {
    fn is_hovered_at(&self, position: Point<Pixels>, window: &Window) -> bool {
        self.is_hovered(window) && self.bounds.contains(&position)
    }
}

/// Flex factors WGPUI's Tailwind-shaped helpers cannot express.
///
/// `Styled` offers `flex_grow()` and `flex_grow_0()`, which set the factor to
/// one and zero. Layouts that share slack in a ratio — table columns sized by
/// their content width, most of all — need the factor itself.
pub trait FlexExt: Styled + Sized {
    /// Sets how much of the free space this element takes, relative to its
    /// siblings' factors.
    fn flex_grow_by(mut self, factor: f32) -> Self {
        self.style().flex_grow = Some(factor);
        self
    }

    /// Sets how much this element shrinks relative to its siblings.
    fn flex_shrink_by(mut self, factor: f32) -> Self {
        self.style().flex_shrink = Some(factor);
        self
    }

    /// Sets the grow factor to one, spelled the way GPUI used to.
    fn flex_grow_1(self) -> Self {
        self.flex_grow_by(1.)
    }

    /// Sets the shrink factor to one, spelled the way GPUI used to.
    fn flex_shrink_1(self) -> Self {
        self.flex_shrink_by(1.)
    }
}

impl<T: Styled + Sized> FlexExt for T {}

// ── Scroll gestures ─────────────────────────────────────────────────────────

/// The gesture a scroll delta belongs to, remembered across the packets of one
/// swipe.
///
/// A trackpad reports both axes on every packet, and the dominant one can flip
/// from packet to packet on a diagonal swipe. Two elements offering different
/// axes — a horizontal carousel inside a vertical page — would then trade the
/// gesture back and forth. Locking to the axis the gesture started on gives
/// both the same answer for as long as the swipe lasts.
///
/// A packet with no phase (a mouse wheel's line deltas, and every packet on a
/// platform that reports none) never locks, so those deltas pass through whole.
#[derive(Clone, Copy, Debug, Default)]
pub struct OngoingScroll {
    /// The axis the current gesture is locked to, once one has won.
    axis_lock: Option<Axis>,
    /// The phase the lock was last updated for, so a repeated packet of one
    /// phase does not re-decide the axis.
    locked_for: Option<TouchPhase>,
}

impl OngoingScroll {
    /// Removes the component of `delta` that crosses the locked axis.
    ///
    /// Call this once per packet, before acting on `delta`. `Started` begins a
    /// new gesture and clears the lock; `Moved` is where a lock is first taken,
    /// from whichever axis is larger in this packet. Ties leave the delta alone
    /// and keep waiting, because a perfectly diagonal packet says nothing about
    /// which axis the user meant.
    pub fn filter(&mut self, delta: &mut Point<Pixels>, touch_phase: TouchPhase) {
        match touch_phase {
            TouchPhase::Started => {
                self.axis_lock = None;
                self.locked_for = Some(touch_phase);
                return;
            }
            TouchPhase::Moved => {}
            TouchPhase::Ended => return,
        }

        if self.axis_lock.is_none() {
            // Only the phase that carries movement can decide the axis; a `Started`
            // packet is zero by construction.
            self.axis_lock = match delta.x.abs().to_f32().total_cmp(&delta.y.abs().to_f32()) {
                std::cmp::Ordering::Greater => Some(Axis::Horizontal),
                std::cmp::Ordering::Less => Some(Axis::Vertical),
                std::cmp::Ordering::Equal => None,
            };
        }
        self.locked_for = Some(touch_phase);

        match self.axis_lock {
            Some(Axis::Horizontal) => delta.y = px(0.),
            Some(Axis::Vertical) => delta.x = px(0.),
            None => {}
        }
    }
}

// ── Geometry ────────────────────────────────────────────────────────────────

/// A `Size` read as the point holding the same two numbers.
///
/// WGPUI hands back as a `Size` several quantities gpui-pre carried as a
/// `Point`, and a scroll handle's maximum offset is the one that bites: a
/// caller clamps a drag against it, or asks how far the view can travel along
/// one axis, and reads `x`/`y` to do it. Nothing is being resized — the same
/// pair of numbers is moving between two types that both hold a width and a
/// height — so the conversion is a rename and not an arithmetic.
pub fn size_as_point(size: Size<Pixels>) -> Point<Pixels> {
    Point::new(size.width, size.height)
}

// ── Text layouts ────────────────────────────────────────────────────────────

/// The wrapped lines of `text_layout`, in the order they paint.
///
/// WGPUI hands a layout's lines out one byte index at a time and keeps the list
/// to itself, so the walk steps by each line's own byte length plus the newline
/// that ended it — the same step that backend's own index lookups take between
/// lines. Each step costs a scan from the first line, which is affordable
/// because the number of lines in a paragraph is small next to the number of
/// glyphs a caller painting a line at a time would otherwise walk.
pub fn line_layouts(text_layout: &TextLayout) -> Vec<Arc<WrappedLineLayout>> {
    let len = text_layout.len();
    let mut lines = Vec::new();
    let mut line_start = 0;
    while line_start <= len {
        let Some(line) = text_layout.line_layout_for_index(line_start) else {
            break;
        };
        line_start += line.len() + 1;
        lines.push(line);
    }
    lines
}

// ── Virtual lists ───────────────────────────────────────────────────────────

/// The chat-log scrolling and re-measurement `ListState` used to carry itself.
///
/// WGPUI's `ListState` already knows how to keep a tail in view: under
/// `ListAlignment::Bottom` it stores no scroll offset while the list sits at its
/// end, that absence resolves to one past the last item, and the latch re-arms
/// itself whenever a scroll lands on the bottom. What it will not do is say so.
/// `alignment` is a private field with no setter, `logical_scroll_top` has no
/// way to be put back to unset, and there is no query for any of it — so the
/// latch is read back out of the offset it resolves to, and re-armed by asking
/// for an offset past the last item, which `scroll_to` clamps to the same place.
///
/// Re-measurement is the other half. gpui-pre's `remeasure_items` turned a range
/// of items back into unmeasured ones while keeping their old heights as a hint
/// *and* re-armed the full-measure latch, in one call. WGPUI splits that across
/// `splice` and `measure_all`, and its unmeasured item carries no height, so the
/// two are run here in the order that keeps the scroll position meaningful.
pub trait ListStateExt {
    /// Marks `range` of the list for re-measurement, keeping the reader where
    /// they are.
    fn remeasure_items(&mut self, range: Range<usize>);

    /// Marks every item for re-measurement.
    fn remeasure(&mut self);

    /// Scrolls to the last item and resumes tail following.
    fn scroll_to_end(&mut self);

    /// Whether the scroll top is at the end of the list, or `None` before the
    /// list has been laid out and the question has no answer yet.
    fn is_scrolled_to_end(&self) -> Option<bool>;

    /// Whether the list is pinned to its tail.
    ///
    /// A list that has not been laid out yet counts as pinned, which is what a
    /// chat log wants: it opens on its newest message.
    fn is_following_tail(&self) -> bool;
}

impl ListStateExt for ListState {
    fn remeasure_items(&mut self, range: Range<usize>) {
        let count = self.item_count();
        let start = range.start.min(count);
        let end = range.end.min(count);
        if start == end {
            return;
        }

        // `splice` is what turns items back into unmeasured ones, but it treats
        // the range as *replaced* and rewinds a scroll offset that falls inside
        // it — a reader partway down the list would be thrown back to the
        // range's start on every update. The item the reader is on is therefore
        // spliced on its own first, while the items above it still hold their
        // heights; the offset restored at that moment describes the reader's
        // position under the old heights, and the two halves either side of that
        // item are spliced afterwards, which leave the offset alone. The reader
        // keeps their pixel position while the content above them reflows.
        let scroll_top = self.logical_scroll_top();
        if (start..end).contains(&scroll_top.item_ix) {
            let reader = scroll_top.item_ix;
            self.splice(reader..reader + 1, 1);
            self.scroll_to(scroll_top);
            self.splice(start..reader, reader - start);
            self.splice(reader + 1..end, end - reader - 1);
        } else {
            self.splice(start..end, end - start);
        }

        // `measure_all` consumes the state it is called on and arms a one-shot
        // latch that has the next layout measure every item, on screen or not.
        // Without it only the visible items would be re-measured and the list's
        // height would grow back as the reader scrolled through it. A clone is
        // the same list, so assigning it back re-arms the latch on the caller's
        // own handle.
        *self = self.clone().measure_all();
    }

    fn remeasure(&mut self) {
        self.remeasure_items(0..self.item_count());
    }

    fn scroll_to_end(&mut self) {
        // `usize::MAX` is clamped to the item count and a zero offset kept,
        // which is exactly the offset the tail latch resolves to. The next
        // layout sees an offset past the last item and re-arms the latch.
        self.scroll_to(ListOffset {
            item_ix: usize::MAX,
            offset_in_item: px(0.),
        });
    }

    fn is_scrolled_to_end(&self) -> Option<bool> {
        if self.viewport_bounds().size.height <= px(0.) {
            return None;
        }

        // The tail latch resolves to an offset at the item *count*, one past the
        // last item, and `scroll_to` clamps to the same place — so an offset
        // there is what "at the end" looks like from the outside.
        Some(self.logical_scroll_top().item_ix >= self.item_count())
    }

    fn is_following_tail(&self) -> bool {
        self.is_scrolled_to_end().unwrap_or(true)
    }
}

// ── Reduced motion ──────────────────────────────────────────────────────────

/// Base's view of the system's reduced-motion preference.
#[derive(Default)]
struct ReducedMotion {
    reduce: bool,
}

impl Global for ReducedMotion {}

/// Supplies the reduced-motion flag Base's transitions consult.
///
/// WGPUI has no such flag, so Base owns it. It is a per-application global
/// rather than a field: every caller of `reduce_motion` reads it through `App`,
/// the same way it would a flag GPUI owned.
pub trait ReduceMotionExt {
    /// Whether animation should be reduced to its end state.
    fn reduce_motion(&self) -> bool;

    /// Sets the flag. An application that calls this owns the flag from then on;
    /// see [`crate::reduce_motion`] for how Base's platform readers defer to it.
    fn set_reduce_motion(&mut self, reduce: bool);
}

impl ReduceMotionExt for App {
    fn reduce_motion(&self) -> bool {
        self.try_global::<ReducedMotion>().is_some_and(|m| m.reduce)
    }

    fn set_reduce_motion(&mut self, reduce: bool) {
        self.default_global::<ReducedMotion>().reduce = reduce;
    }
}

// ── Shaped lines ────────────────────────────────────────────────────────────

/// Paints a shaped line with the alignment and alignment width GPUI's own
/// `paint` used to take.
///
/// WGPUI's `ShapedLine::paint` leaves the line's alignment to the caller of
/// `paint_line`, which it always hands `TextAlign::Left` and no alignment
/// width, so it can only draw a line flush with its own origin. Text that is
/// centred or right-aligned inside a wider box needs the line moved instead.
/// Shifting the origin by the leftover each alignment leaves on its side puts
/// the glyphs exactly where GPUI put them, because GPUI's arithmetic for a
/// shaped line reduced to the same shift.
pub fn paint_shaped_line(
    line: &ShapedLine,
    origin: Point<Pixels>,
    line_height: Pixels,
    align: TextAlign,
    align_width: Option<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Result<()> {
    let origin = aligned_origin(line, origin, align, align_width);
    line.paint(origin, line_height, window, cx)
}

/// [`paint_shaped_line`] for the line's background.
pub fn paint_shaped_line_background(
    line: &ShapedLine,
    origin: Point<Pixels>,
    line_height: Pixels,
    align: TextAlign,
    align_width: Option<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Result<()> {
    let origin = aligned_origin(line, origin, align, align_width);
    line.paint_background(origin, line_height, window, cx)
}

/// Moves `origin` left or right so painting left-aligned there lands the line
/// where `align` inside `align_width` would.
fn aligned_origin(
    line: &ShapedLine,
    origin: Point<Pixels>,
    align: TextAlign,
    align_width: Option<Pixels>,
) -> Point<Pixels> {
    let leftover = align_width.unwrap_or(line.width) - line.width;
    match align {
        TextAlign::Left => origin,
        TextAlign::Center => origin + Point::new(leftover / 2., px(0.)),
        TextAlign::Right => origin + Point::new(leftover, px(0.)),
    }
}

// ── SVG data ────────────────────────────────────────────────────────────────

/// An element that paints raw SVG bytes as a monochrome icon.
///
/// WGPUI's `Svg` takes an asset path and nothing else: the `data` builder that
/// took the bytes themselves is gone. The bytes do still reach the renderer,
/// through [`Window::paint_svg`], which takes them directly, so an icon that
/// has no asset path is painted from a canvas instead. `key` stands in for the
/// path: `paint_svg` caches the rasterized icon in the sprite atlas under the
/// path and the destination size, so two different payloads drawn at the same
/// size must not share a key.
///
/// The value is a canvas rather than an `Svg`, so `with_transformation` is not
/// on it: WGPUI keeps `Transformation`'s scale, translation and rotation
/// private and offers no conversion to the `TransformationMatrix` that
/// `paint_svg` takes, so there is no matrix to pass here.
pub fn svg_data(key: SharedString, data: Arc<[u8]>, color: Hsla) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let _ = window.paint_svg(
                bounds,
                key,
                Some(&data),
                TransformationMatrix::default(),
                color,
                cx,
            );
        },
    )
}

// ── Animation ───────────────────────────────────────────────────────────────

/// Repeating animations that share a phase, GPUI's `Animation::repeat_synced`.
///
/// WGPUI's [`Animation::repeat`] repeats the animation, but its element measures
/// the phase from the `Instant` it was first laid out, so an indicator mounted
/// later starts its own cycle over instead of joining the one already running.
/// The phase here comes from a clock shared by the whole process, handed over
/// through the easing function: the element applies that to its own delta and
/// gives the result to the animator, so every element reading it is at the same
/// point of its cycle. The element's delta is what is ignored — the clock the
/// caller asked for is the only input.
pub trait RepeatSyncedExt {
    /// Repeat this animation in step with every other animation that asks for
    /// the same duration.
    fn repeat_synced(self) -> Animation;
}

impl RepeatSyncedExt for Animation {
    fn repeat_synced(self) -> Animation {
        let duration = self.duration.as_secs_f32();
        self.repeat().with_easing(move |_| synced_phase(duration))
    }
}

/// How far into a `duration` cycle the shared clock is, in `0.0..1.0`.
///
/// The clock starts at the first call, so an application that never asks for a
/// synced animation never reads one.
fn synced_phase(duration: f32) -> f32 {
    static CLOCK: OnceLock<Instant> = OnceLock::new();
    // A zero or non-finite duration has no cycle to report a phase of, and the
    // animation element rejects a delta outside `0.0..=1.0`.
    if !duration.is_finite() || duration <= 0. {
        return 0.;
    }
    let elapsed = CLOCK.get_or_init(Instant::now).elapsed().as_secs_f32();
    (elapsed / duration) % 1.
}

// ── Spring physics ──────────────────────────────────────────────────────────

/// A scalar that a spring can move to.
///
/// The spring solves in `f32` and lets the target say what that number means:
/// [`f32`] is opacity, [`Pixels`] is a distance. `resolve` is called every
/// frame, including while the spring is at rest, so it must be cheap.
pub trait SpringTarget {
    /// What the caller wants back — usually `Self`.
    type Output;

    /// Where the spring is heading, in the spring's own units.
    fn target(&self) -> f32;

    /// The caller's value for a solved position.
    fn resolve(&self, position: f32) -> Self::Output;
}

impl SpringTarget for f32 {
    type Output = f32;

    fn target(&self) -> f32 {
        *self
    }

    fn resolve(&self, position: f32) -> f32 {
        position
    }
}

impl SpringTarget for Pixels {
    type Output = Pixels;

    fn target(&self) -> f32 {
        self.to_f32()
    }

    fn resolve(&self, position: f32) -> Pixels {
        px(position)
    }
}

/// The stiffness, damping and mass a spring is solved with.
///
/// This is the physical parameterization, not a duration: `stiffness` pulls
/// toward the target, `damping` resists motion, and `mass` divides both. A
/// caller wanting a duration in seconds converts it — see
/// [`crate::motion::Spring::config`], which reads a response time and a damping
/// ratio and derives these.
#[derive(Clone, Copy, Debug)]
pub struct SpringConfig {
    /// How hard the spring pulls toward its target.
    stiffness: f32,
    /// How much the spring resists moving.
    damping: f32,
    /// How much the spring resists being moved.
    mass: f32,
}

/// Where a spring is and how fast it is going.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpringState {
    /// Distance from the origin, in the spring's units.
    pub position: f32,
    /// Units per second.
    pub velocity: f32,
}

impl SpringConfig {
    /// A spring with the given physical parameters.
    ///
    /// `stiffness` and `mass` are positive; `damping` is the damping
    /// *coefficient*, where `2 * sqrt(stiffness * mass)` is critical. A
    /// non-positive `mass` or `stiffness` leaves the spring unable to move, which
    /// [`Self::step`] handles by holding still rather than dividing by zero.
    pub const fn new(stiffness: f32, damping: f32, mass: f32) -> Self {
        Self {
            stiffness,
            damping,
            mass,
        }
    }

    /// Advances `state` toward `target` by `elapsed` seconds.
    ///
    /// The step is the closed-form solution of the damped harmonic oscillator
    /// rather than a numerical integration of it. That costs one `exp` and one
    /// `sqrt` per step and buys two things a fixed integrator needs care to get:
    /// the result does not depend on how the frame time was sliced, so a dropped
    /// frame neither overshoots nor goes unstable, and a spring at rest on a new
    /// target arrives exactly rather than asymptotically.
    ///
    /// A non-finite or negative `elapsed` holds the spring where it is.
    pub fn step(&self, state: SpringState, target: f32, elapsed: f32) -> SpringState {
        let SpringState { position, velocity } = state;
        if !elapsed.is_finite() || elapsed <= 0. || !(self.mass > 0.) || !(self.stiffness > 0.) {
            return state;
        }

        let displacement = position - target;
        if !displacement.is_finite() || !velocity.is_finite() {
            // A spring that has been handed a NaN can never recover by stepping, so
            // it is snapped rather than propagated.
            return SpringState {
                position: target,
                velocity: 0.,
            };
        }

        let omega = (self.stiffness / self.mass).sqrt();
        let zeta = self.damping / (2. * (self.stiffness * self.mass).sqrt());
        let (displacement, velocity) = solve(displacement, velocity, omega, zeta, elapsed);
        let next = SpringState {
            position: target + displacement,
            velocity,
        };
        if next.position.is_finite() && next.velocity.is_finite() {
            next
        } else {
            SpringState {
                position: target,
                velocity: 0.,
            }
        }
    }

    /// Whether a spring at `state` has arrived at `target`.
    ///
    /// Both the remaining distance and the speed have to be under `epsilon`: a
    /// spring that is momentarily passing through its target at speed has not
    /// settled, and stopping it there would drop the overshoot the caller asked
    /// for by choosing a spring over an easing curve.
    pub fn is_settled(&self, state: SpringState, target: f32, epsilon: f32) -> bool {
        (state.position - target).abs() < epsilon && state.velocity.abs() < epsilon
    }
}

/// The closed-form displacement and velocity of a damped spring after
/// `elapsed`, given its initial displacement from the target and velocity.
///
/// The three regimes — underdamped, critically damped, overdamped — are
/// separate solutions of the same equation, chosen by the damping ratio
/// `zeta = damping / (2 * sqrt(stiffness * mass))`. They are written out rather
/// than unified through complex arithmetic so the critically damped case,
/// which is what a caller gets from a damping ratio of exactly 1, does not
/// depend on a cancellation that a floating point `sqrt` of a near-zero
/// discriminant would not deliver.
fn solve(x0: f32, v0: f32, omega: f32, zeta: f32, t: f32) -> (f32, f32) {
    // A spring with no damping at all is a harmonic oscillator; give it a trace
    // of damping so the decay term below stays a number below one.
    let zeta = zeta.max(0.);
    if zeta < 1. {
        let damped = omega * (1. - zeta * zeta).sqrt();
        let decay = (-zeta * omega * t).exp();
        let (sin, cos) = (damped * t).sin_cos();
        let a = x0;
        let b = (v0 + zeta * omega * x0) / damped;

        let displacement = decay * (a * cos + b * sin);
        let velocity =
            decay * ((b * damped - zeta * omega * a) * cos - (a * damped + zeta * omega * b) * sin);
        (displacement, velocity)
    } else if zeta == 1. {
        let decay = (-omega * t).exp();
        let c = v0 + omega * x0;

        let displacement = (x0 + c * t) * decay;
        let velocity = (c - omega * (x0 + c * t)) * decay;
        (displacement, velocity)
    } else {
        let root = omega * (zeta * zeta - 1.).sqrt();
        let slow = -zeta * omega + root;
        let fast = -zeta * omega - root;
        // Coefficients from the initial displacement and velocity.
        let a = (v0 - fast * x0) / (slow - fast);
        let b = x0 - a;

        let displacement = a * (slow * t).exp() + b * (fast * t).exp();
        let velocity = a * slow * (slow * t).exp() + b * fast * (fast * t).exp();
        (displacement, velocity)
    }
}

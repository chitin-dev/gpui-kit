use gpui::{App, Bounds, IntoElement, ParentElement, Pixels, Styled as _, Window, canvas};

use crate::TextSelectionScopeId;

/// Extends a GPUI element with text-selection scoping and, for a parent,
/// post-layout prepaint observation.
pub trait ElementExt: IntoElement + Sized {
    /// Marks this element subtree as belonging to a text-selection scope.
    ///
    /// The scope marker is transparent and takes any element, so the method is
    /// on every element rather than only on a parent: an `AnimationElement`
    /// wraps a subtree without being a parent itself, and a scope still has to
    /// go around it.
    fn text_selection_scope(self, scope: TextSelectionScopeId) -> impl IntoElement {
        crate::text_selection::text_selection_scope(scope, self)
    }

    /// Invokes `callback` during prepaint with this element's resolved bounds.
    fn on_prepaint<F>(self, callback: F) -> Self
    where
        Self: ParentElement,
        F: FnOnce(Bounds<Pixels>, &mut Window, &mut App) + 'static,
    {
        self.child(
            canvas(
                move |bounds, window, cx| callback(bounds, window, cx),
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
    }
}

impl<T: IntoElement> ElementExt for T {}

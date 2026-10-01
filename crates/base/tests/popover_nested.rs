use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, point, px,
};
use gpui_base::{GlobalState, Popover};
use std::{cell::RefCell, rc::Rc};

struct Harness(Rc<RefCell<Vec<bool>>>);

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let changes = Rc::clone(&self.0);
        Popover::new("parent")
            .trigger_with(|_, _, _| div().size(px(40.)).child("Open").into_any_element())
            .content(|_, _, _| {
                div()
                    .debug_selector(|| "parent-content".into())
                    .size(px(100.))
            })
            .on_open_change(move |open, _, _| changes.borrow_mut().push(*open))
    }
}

#[gpui::test]
fn child_popup_keeps_parent_open_on_outside_press(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_base::init);
    let changes = Rc::new(RefCell::new(Vec::new()));
    let (_, cx) = cx.add_window_view({
        let changes = Rc::clone(&changes);
        move |_, _| Harness(changes)
    });
    cx.update(|window, cx| window.draw(cx).clear());
    cx.simulate_click(point(px(20.), px(20.)), Default::default());
    cx.update(|window, cx| window.draw(cx).clear());
    assert!(cx.debug_bounds("parent-content").is_some());
    assert_eq!(&*changes.borrow(), &[true]);

    let child = cx.update(|_, cx| GlobalState::register_deferred_popover(cx));
    cx.simulate_click(point(px(300.), px(300.)), Default::default());
    cx.update(|window, cx| window.draw(cx).clear());
    assert!(cx.debug_bounds("parent-content").is_some());
    assert_eq!(&*changes.borrow(), &[true]);

    drop(child);
    cx.simulate_click(point(px(300.), px(300.)), Default::default());
    cx.update(|window, cx| window.draw(cx).clear());
    assert_eq!(&*changes.borrow(), &[true, false]);
}

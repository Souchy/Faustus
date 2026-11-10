pub mod input;

use std::{cell::Cell, rc::Rc};

use gpui_kit::{
    Div, ElementId, MouseButton, MouseMoveEvent, Pixels, Point, ScrollHandle, Stateful, div,
    prelude::*, px,
};

pub(crate) struct RowLimit {
    limit: usize,
}

impl Default for RowLimit {
    fn default() -> Self {
        Self { limit: 50 }
    }
}

impl RowLimit {
    pub(crate) fn visible(&self, total: usize) -> usize {
        self.limit.min(total)
    }

    pub(crate) fn show_more(&mut self) {
        self.limit = self.limit.saturating_add(50);
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Default)]
struct ScrollDrag {
    position: Cell<Option<Point<Pixels>>>,
}

impl ScrollDrag {
    fn offset(
        &self,
        event: &MouseMoveEvent,
        mut offset: Point<Pixels>,
        max_offset: Point<Pixels>,
    ) -> Option<Point<Pixels>> {
        if event.pressed_button != Some(MouseButton::Middle) {
            self.position.set(None);
            return None;
        }
        let previous = self.position.get()?;
        self.position.set(Some(event.position));
        offset.y = (offset.y + event.position.y - previous.y).clamp(-max_offset.y, px(0.));
        Some(offset)
    }
}

#[derive(Default)]
pub(crate) struct TableScroll {
    handle: ScrollHandle,
    drag: Rc<ScrollDrag>,
}

impl TableScroll {
    pub(crate) fn reset(&self) {
        self.handle.set_offset(Point::default());
        self.drag.position.set(None);
    }

    pub(crate) fn container(&self, id: impl Into<ElementId>) -> Stateful<Div> {
        let drag_start = self.drag.clone();
        let drag_move = self.drag.clone();
        let drag_end = self.drag.clone();
        let drag_end_outside = self.drag.clone();
        let handle = self.handle.clone();

        div()
            .id(id)
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .track_scroll(&self.handle)
            .on_mouse_down(MouseButton::Middle, move |event, _, context| {
                drag_start.position.set(Some(event.position));
                context.stop_propagation();
            })
            .on_mouse_move(move |event, window, context| {
                if let Some(offset) = drag_move.offset(event, handle.offset(), handle.max_offset())
                {
                    handle.set_offset(offset);
                    window.refresh();
                    context.stop_propagation();
                }
            })
            .on_mouse_up(MouseButton::Middle, move |_, _, _| {
                drag_end.position.set(None);
            })
            .on_mouse_up_out(MouseButton::Middle, move |_, _, _| {
                drag_end_outside.position.set(None);
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{RowLimit, ScrollDrag};
    use gpui_kit::{MouseButton, MouseMoveEvent, point, px};

    #[test]
    fn row_limit_adds_batches_and_clamps_to_available_rows() {
        let mut limit = RowLimit::default();
        assert_eq!(limit.visible(0), 0);
        assert_eq!(limit.visible(12), 12);
        assert_eq!(limit.visible(125), 50);
        limit.show_more();
        assert_eq!(limit.visible(125), 100);
        limit.show_more();
        assert_eq!(limit.visible(125), 125);
        limit.reset();
        assert_eq!(limit.visible(125), 50);
    }

    #[test]
    fn middle_drag_scrolls_and_clamps_to_content() {
        let drag = ScrollDrag::default();
        let limit = point(px(0.), px(300.));
        drag.position.set(Some(point(px(100.), px(120.))));
        let mut event = MouseMoveEvent {
            position: point(px(100.), px(60.)),
            pressed_button: Some(MouseButton::Middle),
            ..Default::default()
        };
        let offset = drag.offset(&event, point(px(0.), px(-80.)), limit).unwrap();
        assert_eq!(offset.y, px(-140.));
        event.position.y = px(-500.);
        let bottom = drag.offset(&event, offset, limit).unwrap();
        assert_eq!(bottom.y, px(-300.));
        event.position.y = px(500.);
        let top = drag.offset(&event, bottom, limit).unwrap();
        assert_eq!(top.y, px(0.));
    }

    #[test]
    fn drag_requires_middle_press_inside_and_stops_on_release() {
        let drag = ScrollDrag::default();
        let offset = point(px(0.), px(-80.));
        let limit = point(px(0.), px(300.));
        let mut event = MouseMoveEvent {
            position: point(px(100.), px(60.)),
            pressed_button: Some(MouseButton::Middle),
            ..Default::default()
        };
        assert!(drag.offset(&event, offset, limit).is_none());
        drag.position.set(Some(point(px(100.), px(120.))));
        event.pressed_button = Some(MouseButton::Left);
        assert!(drag.offset(&event, offset, limit).is_none());
        assert!(drag.position.get().is_none());
        event.pressed_button = Some(MouseButton::Middle);
        assert!(drag.offset(&event, offset, limit).is_none());
        drag.position.set(Some(point(px(100.), px(120.))));
        event.pressed_button = None;
        assert!(drag.offset(&event, offset, limit).is_none());
        assert!(drag.position.get().is_none());
    }
}

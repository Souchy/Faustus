use gpui_kit::component::{
    Sizable,
    input::{Input, InputState},
};
use gpui_kit::{Entity, div, prelude::*, px};

pub(crate) fn render(state: &Entity<InputState>, width: f32) -> impl IntoElement {
    div()
        .w(px(width))
        .child(Input::new(state).small().cleanable(true))
}

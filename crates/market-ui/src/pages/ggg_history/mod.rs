use gpui_kit::{div, prelude::*, px, rgb};

use crate::{MarketView, format_side_values, short_item_id};

pub(crate) fn render(view: &MarketView) -> impl IntoElement {
    let empty_state = view.error.clone().unwrap_or_else(|| {
        view.selected_league.as_ref().map_or_else(
            || "Choose a league to view GGG history.".to_owned(),
            |league| {
                format!(
                    "No stored GGG aggregate rows for {}. The feed begins at retained history, so current data may take time to arrive.",
                    league.name
                )
            },
        )
    });

    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .flex()
                .gap_6()
                .border_b_1()
                .border_color(rgb(0x334347))
                .pb_3()
                .text_sm()
                .text_color(rgb(0xa6b8b5))
                .child(div().w(px(120.)).child("LEAGUE"))
                .child(div().flex_1().child("INSTRUMENT"))
                .child(div().w(px(220.)).child("VOLUME BY SIDE"))
                .child(div().w(px(280.)).child("RAW RATIO RANGE"))
                .child(div().w(px(120.)).child("PAGE CURSOR")),
        )
        .child(
            div()
                .id("exchange-market-list")
                .flex_1()
                .flex()
                .flex_col()
                .overflow_scroll()
                .children(view.markets.iter().map(|market| {
                    let pair = market
                        .market_pair
                        .iter()
                        .map(|item| short_item_id(item))
                        .collect::<Vec<_>>()
                        .join(" / ");
                    let volume = format_side_values(&market.volume_traded);
                    let low = format_side_values(&market.lowest_ratio);
                    let high = format_side_values(&market.highest_ratio);

                    div()
                        .flex()
                        .items_center()
                        .gap_6()
                        .border_b_1()
                        .border_color(rgb(0x28383b))
                        .py_1()
                        .text_sm()
                        .child(div().w(px(120.)).child(market.league.clone()))
                        .child(div().flex_1().child(pair))
                        .child(div().w(px(220.)).child(volume))
                        .child(
                            div()
                                .w(px(280.))
                                .child(format!("low [{low}]  high [{high}]")),
                        )
                        .child(div().w(px(120.)).child(market.next_change_id.to_string()))
                }))
                .items_center()
                .justify_center()
                .text_color(rgb(0xa6b8b5))
                .when(view.markets.is_empty(), |list| list.child(empty_state)),
        )
}

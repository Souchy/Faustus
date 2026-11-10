use gpui_kit::{div, prelude::*, px, rgb};

use crate::MarketView;

pub(crate) fn render(view: &MarketView) -> impl IntoElement {
    let fetched_at = view
        .price_overview
        .as_ref()
        .map(|overview| overview.fetched_at.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "not available".to_owned());
    let primary_currency = view
        .price_overview
        .as_ref()
        .map(|overview| overview.primary.name.clone())
        .unwrap_or_else(|| "Chaos Orb".to_owned());
    let league_name = view
        .selected_league
        .as_ref()
        .map(|league| league.name.as_str())
        .unwrap_or("the selected league");
    let empty_state = view.price_error.clone().unwrap_or_else(|| {
        format!("Current poe.ninja prices for {league_name} are not available yet.")
    });

    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .flex()
                .justify_between()
                .text_sm()
                .text_color(rgb(0xa6b8b5))
                .child(format!(
                    "poe.ninja current exchange estimates  |  Quoted in {primary_currency}  |  Updated {fetched_at}"
                )),
        )
        .child(
            div()
                .flex()
                .gap_6()
                .border_b_1()
                .border_color(rgb(0x334347))
                .pb_2()
                .text_sm()
                .text_color(rgb(0xa6b8b5))
                .child(div().flex_1().child("ITEM"))
                .child(div().w(px(150.)).child("ESTIMATE"))
                .child(div().w(px(150.)).child("VOLUME"))
                .child(div().w(px(210.)).child("DOMINANT PAIR"))
                .child(div().w(px(110.)).child("TREND")),
        )
        .child(
            div()
                .id("current-exchange-price-list")
                .flex_1()
                .flex()
                .flex_col()
                .overflow_scroll()
                .children(view.price_overview.iter().flat_map(|overview| {
                    overview.lines.iter().map(|line| {
                        let dominant_pair = match (
                            line.max_volume_rate.as_deref(),
                            line.max_volume_currency.as_deref(),
                        ) {
                            (Some(rate), Some(currency)) => format!("{rate} {currency}"),
                            _ => "-".to_owned(),
                        };
                        let trend = line
                            .sparkline_change
                            .as_deref()
                            .map(|change| format!("{change}%"))
                            .unwrap_or_else(|| "-".to_owned());
                        let primary_id = overview.primary.id.as_str();

                        div()
                            .flex()
                            .items_center()
                            .gap_6()
                            .border_b_1()
                            .border_color(rgb(0x28383b))
                            .py_1()
                            .text_sm()
                            .child(div().flex_1().child(line.name.clone()))
                            .child(div().w(px(150.)).child(format!(
                                "{} {primary_id}",
                                line.primary_value.as_deref().unwrap_or("-")
                            )))
                            .child(div().w(px(150.)).child(format!(
                                "{} {primary_id}",
                                line.volume_primary_value.as_deref().unwrap_or("-")
                            )))
                            .child(div().w(px(210.)).child(dominant_pair))
                            .child(div().w(px(110.)).child(trend))
                    })
                }))
                .items_center()
                .justify_center()
                .text_color(rgb(0xa6b8b5))
                .when(
                    view.price_overview
                        .as_ref()
                        .is_none_or(|overview| overview.lines.is_empty()),
                    |list| list.child(empty_state),
                ),
        )
}

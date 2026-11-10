use std::collections::BTreeMap;

use gpui_kit::{Context, div, prelude::*, px, rgb};
use market_core::{CurrentExchangePriceOverview, GemMarketLine};

use crate::{MarketView, pages::time_range::TimeRangeSelection};

const LOW_CONFIDENCE_MIN_SAMPLES: u64 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GemStrategy {
    Level,
    Quality,
    CorruptLevel,
    CorruptQuality,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GemSortColumn {
    Name,
    BuyVariant,
    BuyPrice,
    SellVariant,
    SellPrice,
    Materials,
    Net,
    Roi,
    Listings,
}

impl GemSortColumn {
    fn ascending_by_default(self) -> bool {
        matches!(self, Self::Name | Self::BuyVariant | Self::SellVariant)
    }
}

impl GemStrategy {
    const ALL: [Self; 4] = [
        Self::Level,
        Self::Quality,
        Self::CorruptLevel,
        Self::CorruptQuality,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Level => "Level flip",
            Self::Quality => "Add quality",
            Self::CorruptLevel => "Vaal +1 level",
            Self::CorruptQuality => "Vaal +3 quality",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Level => {
                "Buy level 1, sell the highest uncorrupted level. Leveling time excluded; Lens is optional."
            }
            Self::Quality => {
                "Compare max-level 0% and 20% quality; cost includes 20 Gemcutter's Prisms."
            }
            Self::CorruptLevel => {
                "Compare max-level 20% to corrupted +1 level. Profit is conditional on that outcome."
            }
            Self::CorruptQuality => {
                "Compare max-level 20% to corrupted +3% quality. Profit is conditional on that outcome."
            }
        }
    }
}

struct GemOpportunity {
    name: String,
    source_variant: String,
    target_variant: String,
    source_price: f64,
    target_price: f64,
    material_cost: f64,
    profit: f64,
    roi_percent: f64,
    source_listings: u64,
    target_listings: u64,
    low_confidence: bool,
}

#[derive(Clone, Copy)]
struct GemAnalysisInputs {
    gcp_price: Option<f64>,
    vaal_price: Option<f64>,
    lens_price: Option<f64>,
    use_lens: bool,
    lens_count: u8,
    time_range: TimeRangeSelection,
}

fn exchange_price(
    overview: Option<&CurrentExchangePriceOverview>,
    id: &str,
    time_range: TimeRangeSelection,
) -> Option<f64> {
    let line = overview?.lines.iter().find(|line| line.id == id)?;
    price_at_range(
        line.primary_value.as_deref()?.parse().ok(),
        &line.sparkline,
        line.sparkline_change.as_deref(),
        time_range,
        true,
    )
}

fn opportunities(
    gems: Option<&[GemMarketLine]>,
    strategy: GemStrategy,
    inputs: GemAnalysisInputs,
) -> Vec<GemOpportunity> {
    let Some(gems) = gems else {
        return Vec::new();
    };
    if strategy == GemStrategy::Level
        && inputs.use_lens
        && inputs
            .lens_price
            .is_none_or(|price| !price.is_finite() || price <= 0.0)
    {
        return Vec::new();
    }
    if strategy == GemStrategy::Quality && inputs.gcp_price.is_none() {
        return Vec::new();
    }
    if matches!(
        strategy,
        GemStrategy::CorruptLevel | GemStrategy::CorruptQuality
    ) && inputs.vaal_price.is_none()
    {
        return Vec::new();
    }

    let mut groups = BTreeMap::<&str, Vec<&GemMarketLine>>::new();
    for gem in gems {
        groups.entry(&gem.name).or_default().push(gem);
    }

    groups
        .into_iter()
        .filter_map(|(name, variants)| {
            let (source, target, material_cost) = match strategy {
                GemStrategy::Level => {
                    let source = find_variant(&variants, 1, 0, false)?;
                    let max_level = variants
                        .iter()
                        .filter(|gem| !gem.corrupted && gem_quality(gem) == 0)
                        .filter_map(|gem| gem.gem_level)
                        .max()?;
                    let target = find_variant(&variants, max_level, 0, false)?;
                    if max_level <= 1 {
                        return None;
                    }
                    let lens_cost = if inputs.use_lens {
                        inputs.lens_price? * f64::from(inputs.lens_count)
                    } else {
                        0.0
                    };
                    (source, target, lens_cost)
                }
                GemStrategy::Quality => {
                    let level = variants
                        .iter()
                        .filter(|gem| !gem.corrupted)
                        .filter_map(|gem| gem.gem_level)
                        .max()?;
                    let source = find_variant(&variants, level, 0, false)?;
                    let target = find_variant(&variants, level, 20, false)?;
                    (source, target, inputs.gcp_price? * 20.0)
                }
                GemStrategy::CorruptLevel | GemStrategy::CorruptQuality => {
                    let level = variants
                        .iter()
                        .filter(|gem| !gem.corrupted && gem_quality(gem) == 20)
                        .filter_map(|gem| gem.gem_level)
                        .max()?;
                    let source = find_variant(&variants, level, 20, false)?;
                    let (target_level, target_quality) = if strategy == GemStrategy::CorruptLevel {
                        (level.saturating_add(1), 20)
                    } else {
                        (level, 23)
                    };
                    let target = find_variant(&variants, target_level, target_quality, true)?;
                    (source, target, inputs.vaal_price?)
                }
            };

            let source_price = gem_price(source, inputs.time_range, true)?;
            let target_price = gem_price(target, inputs.time_range, false)?;
            let total_cost = source_price + material_cost;
            if total_cost <= 0.0 {
                return None;
            }
            Some(GemOpportunity {
                name: name.to_owned(),
                source_variant: variant_label(source),
                target_variant: variant_label(target),
                source_price,
                target_price,
                material_cost,
                profit: target_price - total_cost,
                roi_percent: (target_price / total_cost - 1.0) * 100.0,
                source_listings: source.listing_count.unwrap_or_default(),
                target_listings: target.listing_count.unwrap_or_default(),
                low_confidence: is_low_confidence(source) || is_low_confidence(target),
            })
        })
        .collect::<Vec<_>>()
}

fn sort_opportunities(rows: &mut Vec<GemOpportunity>, column: GemSortColumn, ascending: bool) {
    rows.sort_by(|left, right| {
        let order = match column {
            GemSortColumn::Name => left.name.cmp(&right.name),
            GemSortColumn::BuyVariant => left.source_variant.cmp(&right.source_variant),
            GemSortColumn::BuyPrice => left.source_price.total_cmp(&right.source_price),
            GemSortColumn::SellVariant => left.target_variant.cmp(&right.target_variant),
            GemSortColumn::SellPrice => left.target_price.total_cmp(&right.target_price),
            GemSortColumn::Materials => left.material_cost.total_cmp(&right.material_cost),
            GemSortColumn::Net => left.profit.total_cmp(&right.profit),
            GemSortColumn::Roi => left.roi_percent.total_cmp(&right.roi_percent),
            GemSortColumn::Listings => left
                .source_listings
                .saturating_add(left.target_listings)
                .cmp(&right.source_listings.saturating_add(right.target_listings)),
        };
        if ascending { order } else { order.reverse() }
    });
    rows.truncate(100);
}

fn matches_search(name: &str, query: &str) -> bool {
    query.is_empty() || name.to_lowercase().contains(query)
}

fn is_low_confidence(gem: &GemMarketLine) -> bool {
    gem.sample_count
        .is_none_or(|count| count < LOW_CONFIDENCE_MIN_SAMPLES)
        || gem
            .listing_count
            .is_none_or(|count| count < LOW_CONFIDENCE_MIN_SAMPLES)
}

fn filter_opportunities(rows: &mut Vec<GemOpportunity>, query: &str, include_low_confidence: bool) {
    rows.retain(|row| {
        matches_search(&row.name, query) && (include_low_confidence || !row.low_confidence)
    });
}

fn find_variant<'a>(
    variants: &[&'a GemMarketLine],
    level: u8,
    quality: u8,
    corrupted: bool,
) -> Option<&'a GemMarketLine> {
    variants.iter().copied().find(|gem| {
        gem.gem_level == Some(level) && gem_quality(gem) == quality && gem.corrupted == corrupted
    })
}

fn gem_quality(gem: &GemMarketLine) -> u8 {
    gem.gem_quality.unwrap_or_default()
}

fn gem_price(gem: &GemMarketLine, time_range: TimeRangeSelection, use_start: bool) -> Option<f64> {
    price_at_range(
        gem.chaos_value.as_deref()?.parse().ok(),
        &gem.sparkline,
        gem.sparkline_change.as_deref(),
        time_range,
        use_start,
    )
}

fn price_at_range(
    current_price: Option<f64>,
    sparkline: &[Option<String>],
    total_change: Option<&str>,
    time_range: TimeRangeSelection,
    use_start: bool,
) -> Option<f64> {
    let current_price = current_price.filter(|price| price.is_finite() && *price >= 0.0)?;
    let days_ago = if use_start {
        time_range.start_days_ago()
    } else {
        time_range.end_days_ago()
    };
    if days_ago == 0 {
        return Some(current_price);
    }
    let latest_index = time_range.point_index(sparkline.len(), 0)?;
    let selected_index = time_range.point_index(sparkline.len(), days_ago)?;
    let selected_change = sparkline[selected_index]
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| {
            (days_ago == 0)
                .then(|| total_change?.parse::<f64>().ok())
                .flatten()
        })?;
    let end_change = sparkline[latest_index]
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| total_change?.parse::<f64>().ok())?;
    let selected_factor = 1.0 + selected_change / 100.0;
    let end_factor = 1.0 + end_change / 100.0;
    if selected_factor <= 0.0 || end_factor <= 0.0 {
        return None;
    }
    let price = current_price * selected_factor / end_factor;
    price.is_finite().then_some(price)
}

fn selectable_cell(
    id: impl Into<gpui_kit::ElementId>,
    value: impl Into<gpui_kit::SharedString>,
) -> impl IntoElement {
    div()
        .text_sm()
        .whitespace_nowrap()
        .child(gpui_kit::base::SelectableText::new(id, value))
}

fn variant_label(gem: &GemMarketLine) -> String {
    format!(
        "Lv{} Q{}{}",
        gem.gem_level.unwrap_or_default(),
        gem_quality(gem),
        if gem.corrupted { " corrupted" } else { "" }
    )
}

pub(crate) fn render(view: &MarketView, cx: &mut Context<MarketView>) -> impl IntoElement {
    let time_range = view.gem_time_range;
    let gcp_price = exchange_price(view.price_overview.as_ref(), "gcp", time_range);
    let vaal_price = exchange_price(view.price_overview.as_ref(), "vaal", time_range);
    let lens_price = view.lens_price_text.parse::<f64>().ok();
    let inputs = GemAnalysisInputs {
        gcp_price,
        vaal_price,
        lens_price,
        use_lens: view.use_facetors_lens,
        lens_count: view.lens_count,
        time_range,
    };
    let mut rows = opportunities(
        view.gem_overview
            .as_ref()
            .map(|overview| overview.gems.as_slice()),
        view.gem_strategy,
        inputs,
    );
    let search_query = view.gem_search.trim().to_lowercase();
    filter_opportunities(&mut rows, &search_query, view.include_low_confidence_gems);
    sort_opportunities(&mut rows, view.gem_sort_column, view.gem_sort_ascending);
    let total_rows = rows.len();
    let visible_rows = view.gem_row_limit.visible(total_rows);
    let rows_empty = rows.is_empty();
    let price_label = |price: Option<f64>| {
        price.map_or_else(
            || "not available".to_owned(),
            |value| format!("{value:.2}c"),
        )
    };
    let empty_state = view.gem_error.clone().unwrap_or_else(|| {
        if view.gem_overview.is_none() {
            format!("Loading gem variants for {}.", selected_league_name(view))
        } else if view.gem_strategy == GemStrategy::Level
            && view.use_facetors_lens
            && lens_price.is_none_or(|price| !price.is_finite() || price <= 0.0)
        {
            "Set the Facetor's Lens unit price to calculate this strategy.".to_owned()
        } else if time_range.start_days_ago() != time_range.end_days_ago() {
            "No matching variants have enough poe.ninja price history for this period.".to_owned()
        } else {
            "No matching variants with listed prices were found for this strategy.".to_owned()
        }
    });

    div()
        .flex_1()
        .flex()
        .flex_col()
        .overflow_hidden()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .children(GemStrategy::ALL.into_iter().map(|strategy| {
                            let selected = view.gem_strategy == strategy;
                            div()
                                .id(format!("gem-strategy-{strategy:?}"))
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.gem_strategy = strategy;
                                    view.reset_gem_rows();
                                    cx.notify();
                                }))
                                .px_2()
                                .py_1()
                                .border_1()
                                .border_color(if selected {
                                    rgb(0x84b9a2)
                                } else {
                                    rgb(0x334347)
                                })
                                .text_sm()
                                .whitespace_nowrap()
                                .child(strategy.label())
                        })),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_sm()
                        .child(format!(
                            "GCP {}  Vaal {}",
                            price_label(gcp_price),
                            price_label(vaal_price)
                        ))
                        .child("Lens")
                        .child(
                            div()
                                .id("lens-toggle")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.use_facetors_lens = !view.use_facetors_lens;
                                    view.reset_gem_rows();
                                    cx.notify();
                                }))
                                .px_2()
                                .border_1()
                                .border_color(if view.use_facetors_lens {
                                    rgb(0x84b9a2)
                                } else {
                                    rgb(0x526568)
                                })
                                .whitespace_nowrap()
                                .child(if view.use_facetors_lens { "On" } else { "Off" }),
                        )
                        .child(crate::pages::components::input::render(
                            &view.lens_price_input,
                            90.,
                        ))
                        .child("x")
                        .child(
                            div()
                                .id("lens-count-decrease")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.lens_count = view.lens_count.saturating_sub(1).max(1);
                                    view.reset_gem_rows();
                                    cx.notify();
                                }))
                                .child("-"),
                        )
                        .child(view.lens_count.to_string())
                        .child(
                            div()
                                .id("lens-count-increase")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.lens_count = view.lens_count.saturating_add(1).min(20);
                                    view.reset_gem_rows();
                                    cx.notify();
                                }))
                                .child("+"),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .flex_wrap()
                .gap_3()
                .text_sm()
                .text_color(rgb(0xa6b8b5))
                .child(crate::pages::components::input::render(
                    &view.gem_search_input,
                    280.,
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child("Start date")
                        .child(
                            div()
                                .id("gem-start-earlier")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    let range = view.gem_time_range;
                                    view.gem_time_range = range.with_start_days_ago(
                                        range.start_days_ago().saturating_add(1),
                                    );
                                    view.reset_gem_rows();
                                    cx.notify();
                                }))
                                .px_3()
                                .py_2()
                                .border_1()
                                .border_color(rgb(0x334347))
                                .child("Earlier"),
                        )
                        .child(
                            div()
                                .px_3()
                                .py_2()
                                .whitespace_nowrap()
                                .child(time_range_label(time_range.start_days_ago())),
                        )
                        .child(
                            div()
                                .id("gem-start-later")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    let range = view.gem_time_range;
                                    view.gem_time_range = range.with_start_days_ago(
                                        range.start_days_ago().saturating_sub(1),
                                    );
                                    view.reset_gem_rows();
                                    cx.notify();
                                }))
                                .px_3()
                                .py_2()
                                .border_1()
                                .border_color(rgb(0x334347))
                                .child("Later"),
                        )
                        .child("End: Now"),
                )
                .child(
                    div()
                        .id("include-low-confidence-gems")
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.include_low_confidence_gems = !view.include_low_confidence_gems;
                            view.reset_gem_rows();
                            cx.notify();
                        }))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(18.))
                                .h(px(18.))
                                .border_1()
                                .border_color(rgb(0x84b9a2))
                                .items_center()
                                .justify_center()
                                .child(if view.include_low_confidence_gems {
                                    "x"
                                } else {
                                    ""
                                }),
                        )
                        .child("Include low confidence"),
                ),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .text_sm()
                .text_color(rgb(0xa6b8b5))
                .child(view.gem_strategy.description())
                .child(
                    "Prices at the selected start/end dates. Vaal outcome returns are conditional.",
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x829895))
                .child("Historical prices are estimates from poe.ninja's seven-point sparkline."),
        )
        .child(
            div().flex_1().min_h(px(0.)).flex().justify_center().child(
                div()
                    .w(px(1040.))
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .border_b_1()
                            .border_color(rgb(0x334347))
                            .pb_2()
                            .text_sm()
                            .text_color(rgb(0xa6b8b5))
                            .child(sortable_header(view, cx, "GEM", GemSortColumn::Name, None))
                            .child(sortable_header(
                                view,
                                cx,
                                "BUY VARIANT",
                                GemSortColumn::BuyVariant,
                                Some(130.),
                            ))
                            .child(sortable_header(
                                view,
                                cx,
                                "BUY",
                                GemSortColumn::BuyPrice,
                                Some(75.),
                            ))
                            .child(sortable_header(
                                view,
                                cx,
                                "SELL VARIANT",
                                GemSortColumn::SellVariant,
                                Some(130.),
                            ))
                            .child(sortable_header(
                                view,
                                cx,
                                "SELL",
                                GemSortColumn::SellPrice,
                                Some(75.),
                            ))
                            .child(sortable_header(
                                view,
                                cx,
                                "MATERIALS",
                                GemSortColumn::Materials,
                                Some(80.),
                            ))
                            .child(sortable_header(
                                view,
                                cx,
                                "NET",
                                GemSortColumn::Net,
                                Some(80.),
                            ))
                            .child(sortable_header(
                                view,
                                cx,
                                "ROI",
                                GemSortColumn::Roi,
                                Some(60.),
                            ))
                            .child(sortable_header(
                                view,
                                cx,
                                "LISTINGS",
                                GemSortColumn::Listings,
                                Some(70.),
                            )),
                    )
                    .child(
                        view.gem_table_scroll
                            .container("gem-opportunities")
                            .children(rows.into_iter().take(visible_rows).map(|opportunity| {
                                let name = opportunity.name.clone();
                                let row_id = format!(
                                    "{}-{}-{}",
                                    opportunity.name,
                                    opportunity.source_variant,
                                    opportunity.target_variant
                                );
                                let profit_color = if opportunity.profit >= 0.0 {
                                    rgb(0x91d5b3)
                                } else {
                                    rgb(0xe28c82)
                                };
                                div()
                                    .flex()
                                    .flex_shrink_0()
                                    .items_center()
                                    .gap_2()
                                    .border_b_1()
                                    .border_color(rgb(0x28383b))
                                    .py_1()
                                    .text_color(rgb(0xa6b8b5))
                                    .child(
                                        div()
                                            .flex_1()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(div().flex_1().overflow_hidden().child(
                                                selectable_cell(format!("gem-name-{row_id}"), name),
                                            ))
                                            .when(opportunity.low_confidence, |name| {
                                                name.text_xs()
                                                    .text_color(rgb(0xe6b96b))
                                                    .child("LOW CONF")
                                            }),
                                    )
                                    .child(div().w(px(130.)).overflow_hidden().child(
                                        selectable_cell(
                                            format!("buy-variant-{row_id}"),
                                            opportunity.source_variant,
                                        ),
                                    ))
                                    .child(div().w(px(75.)).overflow_hidden().child(
                                        selectable_cell(
                                            format!("buy-price-{row_id}"),
                                            format!("{:.2}c", opportunity.source_price),
                                        ),
                                    ))
                                    .child(div().w(px(130.)).overflow_hidden().child(
                                        selectable_cell(
                                            format!("sell-variant-{row_id}"),
                                            opportunity.target_variant,
                                        ),
                                    ))
                                    .child(div().w(px(75.)).overflow_hidden().child(
                                        selectable_cell(
                                            format!("sell-price-{row_id}"),
                                            format!("{:.2}c", opportunity.target_price),
                                        ),
                                    ))
                                    .child(div().w(px(80.)).overflow_hidden().child(
                                        selectable_cell(
                                            format!("materials-{row_id}"),
                                            format!("{:.2}c", opportunity.material_cost),
                                        ),
                                    ))
                                    .child(
                                        div()
                                            .w(px(80.))
                                            .overflow_hidden()
                                            .text_color(profit_color)
                                            .child(selectable_cell(
                                                format!("net-{row_id}"),
                                                format!("{:.2}c", opportunity.profit),
                                            )),
                                    )
                                    .child(div().w(px(60.)).overflow_hidden().child(
                                        selectable_cell(
                                            format!("roi-{row_id}"),
                                            format!("{:.1}%", opportunity.roi_percent),
                                        ),
                                    ))
                                    .child(div().w(px(70.)).overflow_hidden().child(
                                        selectable_cell(
                                            format!("listings-{row_id}"),
                                            format!(
                                                "{}/{}",
                                                opportunity.source_listings,
                                                opportunity.target_listings
                                            ),
                                        ),
                                    ))
                            }))
                            .when(rows_empty, |list| {
                                list.child(
                                    div()
                                        .flex_1()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(empty_state),
                                )
                            }),
                    )
                    .when(total_rows > 0, |table| {
                        table.child(
                            div()
                                .flex()
                                .flex_shrink_0()
                                .items_center()
                                .justify_center()
                                .gap_3()
                                .py_2()
                                .text_sm()
                                .child(format!("Showing {visible_rows} of {total_rows}"))
                                .when(visible_rows < total_rows, |footer| {
                                    footer.child(
                                        gpui_kit::component::button::Button::new("gem-show-more")
                                            .label("Show more")
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.gem_row_limit.show_more();
                                                cx.notify();
                                            })),
                                    )
                                }),
                        )
                    }),
            ),
        )
}

fn selected_league_name(view: &MarketView) -> &str {
    view.selected_league
        .as_ref()
        .map(|league| league.name.as_str())
        .unwrap_or("the selected league")
}

fn time_range_label(days_ago: u8) -> String {
    match days_ago {
        0 => "Now".to_owned(),
        days => format!("{days}d ago"),
    }
}

fn sortable_header(
    view: &MarketView,
    cx: &mut Context<MarketView>,
    title: &str,
    column: GemSortColumn,
    width: Option<f32>,
) -> impl IntoElement {
    let selected = view.gem_sort_column == column;
    let direction = if selected {
        if view.gem_sort_ascending { " ^" } else { " v" }
    } else {
        ""
    };
    let label = format!("{title}{direction}");
    let header = div()
        .id(format!("gem-sort-{column:?}"))
        .on_click(cx.listener(move |view, _, _, cx| {
            if view.gem_sort_column == column {
                view.gem_sort_ascending = !view.gem_sort_ascending;
            } else {
                view.gem_sort_column = column;
                view.gem_sort_ascending = column.ascending_by_default();
            }
            view.reset_gem_rows();
            cx.notify();
        }))
        .px_1()
        .whitespace_nowrap()
        .truncate()
        .child(label);
    if let Some(width) = width {
        header.w(px(width))
    } else {
        header.flex_1()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs_for_test(time_range: TimeRangeSelection) -> GemAnalysisInputs {
        GemAnalysisInputs {
            gcp_price: None,
            vaal_price: None,
            lens_price: None,
            use_lens: false,
            lens_count: 1,
            time_range,
        }
    }

    fn gem(level: u8, quality: u8, corrupted: bool, chaos_value: &str) -> GemMarketLine {
        GemMarketLine {
            name: "Fireball".to_owned(),
            gem_kind: None,
            variant: Some(format!("{level}/{quality}")),
            gem_level: Some(level),
            gem_quality: Some(quality),
            corrupted,
            chaos_value: Some(chaos_value.to_owned()),
            sample_count: Some(12),
            listing_count: Some(12),
            sparkline_change: None,
            sparkline: Vec::new(),
        }
    }

    #[test]
    fn strategies_match_variants_and_subtract_material_costs() {
        let current_range = TimeRangeSelection::new(0, 0);
        let gems = vec![
            gem(1, 0, false, "2"),
            gem(20, 0, false, "40"),
            gem(20, 20, false, "100"),
            gem(21, 20, true, "140"),
            gem(20, 23, true, "120"),
        ];
        let gems = Some(gems.as_slice());

        let level = opportunities(
            gems,
            GemStrategy::Level,
            GemAnalysisInputs {
                gcp_price: None,
                vaal_price: None,
                lens_price: None,
                use_lens: false,
                lens_count: 1,
                time_range: current_range,
            },
        );
        let level_with_lens = opportunities(
            gems,
            GemStrategy::Level,
            GemAnalysisInputs {
                lens_price: Some(5.0),
                use_lens: true,
                lens_count: 2,
                ..inputs_for_test(current_range)
            },
        );
        let quality = opportunities(
            gems,
            GemStrategy::Quality,
            GemAnalysisInputs {
                gcp_price: Some(1.0),
                ..inputs_for_test(current_range)
            },
        );
        let corrupt_level = opportunities(
            gems,
            GemStrategy::CorruptLevel,
            GemAnalysisInputs {
                vaal_price: Some(2.0),
                ..inputs_for_test(current_range)
            },
        );
        let corrupt_quality = opportunities(
            gems,
            GemStrategy::CorruptQuality,
            GemAnalysisInputs {
                vaal_price: Some(2.0),
                ..inputs_for_test(current_range)
            },
        );

        assert_eq!(level[0].profit, 38.0);
        assert_eq!(level_with_lens[0].material_cost, 10.0);
        assert_eq!(level_with_lens[0].profit, 28.0);
        assert_eq!(quality[0].material_cost, 20.0);
        assert_eq!(quality[0].profit, 40.0);
        assert_eq!(corrupt_level[0].profit, 38.0);
        assert_eq!(corrupt_quality[0].profit, 18.0);
    }

    #[test]
    fn historical_prices_use_both_selected_range_endpoints() {
        let sparkline = ["0", "20", "50", "100", "150", "200", "300"]
            .into_iter()
            .map(|value| Some(value.to_owned()))
            .collect::<Vec<_>>();

        let range = TimeRangeSelection::new(3, 1);
        assert_eq!(
            price_at_range(Some(400.0), &sparkline, Some("300"), range, true,),
            Some(200.0)
        );
        assert_eq!(
            price_at_range(Some(400.0), &sparkline, Some("300"), range, false),
            Some(300.0)
        );
        assert_eq!(
            price_at_range(
                Some(400.0),
                &sparkline,
                Some("300"),
                TimeRangeSelection::new(6, 0),
                true,
            ),
            Some(100.0)
        );
    }

    #[test]
    fn opportunities_sort_by_selected_column_and_direction() {
        let opportunity = |name: &str, profit: f64| GemOpportunity {
            name: name.to_owned(),
            source_variant: "Lv1 Q0".to_owned(),
            target_variant: "Lv20 Q0".to_owned(),
            source_price: 1.0,
            target_price: profit + 1.0,
            material_cost: 0.0,
            profit,
            roi_percent: profit * 100.0,
            source_listings: 1,
            target_listings: 2,
            low_confidence: false,
        };
        let mut rows = vec![opportunity("Fireball", 10.0), opportunity("Arc", 20.0)];

        sort_opportunities(&mut rows, GemSortColumn::Net, false);
        assert_eq!(rows[0].name, "Arc");

        sort_opportunities(&mut rows, GemSortColumn::Name, true);
        assert_eq!(rows[0].name, "Arc");
    }

    #[test]
    fn low_sample_variants_are_marked_and_can_be_filtered() {
        let mut source = gem(1, 0, false, "2");
        source.sample_count = Some(2);
        source.listing_count = None;
        let gems = vec![source, gem(20, 0, false, "40")];
        let mut rows = opportunities(
            Some(gems.as_slice()),
            GemStrategy::Level,
            inputs_for_test(TimeRangeSelection::new(0, 0)),
        );

        assert!(rows[0].low_confidence);
        filter_opportunities(&mut rows, "fire", false);
        assert!(rows.is_empty());

        let mut rows = opportunities(
            Some(gems.as_slice()),
            GemStrategy::Level,
            inputs_for_test(TimeRangeSelection::new(0, 0)),
        );
        filter_opportunities(&mut rows, "fire", true);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].low_confidence);
        assert_eq!(rows[0].source_listings, 0);
    }

    #[test]
    fn shared_search_matches_names_without_case_sensitivity() {
        assert!(matches_search("Multistrike Support", "multi"));
        assert!(!matches_search("Multistrike Support", "fireball"));
        assert!(matches_search("Multistrike Support", ""));
    }
}

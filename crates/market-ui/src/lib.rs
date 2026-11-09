#[cfg(not(target_arch = "wasm32"))]
use std::sync::OnceLock;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    App, Bounds, Context, Entity, Render, Subscription, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use market_core::{
    CurrentExchangePriceOverview, ExchangeMarketSnapshot, GemMarketOverview, LeagueOption,
    ServiceStatus,
};
use serde::Serialize;
mod pages;

#[cfg(not(target_arch = "wasm32"))]
static HTTP_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
fn http_runtime() -> &'static tokio::runtime::Runtime {
    HTTP_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to create the HTTP runtime")
    })
}

struct MarketView {
    leagues: Vec<LeagueOption>,
    selected_league: Option<LeagueOption>,
    league_menu_open: bool,
    markets: Vec<ExchangeMarketSnapshot>,
    price_overview: Option<CurrentExchangePriceOverview>,
    price_error: Option<String>,
    gem_overview: Option<GemMarketOverview>,
    gem_error: Option<String>,
    show_prices: bool,
    show_gems: bool,
    gem_strategy: pages::gem_flipping::GemStrategy,
    gem_time_range: pages::time_range::TimeRangeSelection,
    gem_sort_column: pages::gem_flipping::GemSortColumn,
    gem_sort_ascending: bool,
    gem_search: String,
    include_low_confidence_gems: bool,
    gem_search_input: Entity<InputState>,
    gem_table_scroll: pages::components::TableScroll,
    gem_row_limit: pages::components::RowLimit,
    lens_price_input: Entity<InputState>,
    _input_subscriptions: Vec<Subscription>,
    use_facetors_lens: bool,
    lens_price_text: String,
    lens_count: u8,
    error: Option<String>,
}

impl MarketView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let gem_search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search gems")
                .clean_on_escape()
        });
        let lens_price_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Price in Chaos")
                .validate(|value, _| {
                    value.is_empty()
                        || (value
                            .chars()
                            .all(|character| character.is_ascii_digit() || character == '.')
                            && value.chars().filter(|character| *character == '.').count() <= 1)
                })
        });
        let input_subscriptions = vec![
            cx.subscribe_in(&gem_search_input, window, |view, input, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    view.gem_search = input.read(cx).value().to_string();
                    view.reset_gem_rows();
                    cx.notify();
                }
            }),
            cx.subscribe_in(&lens_price_input, window, |view, input, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    view.lens_price_text = input.read(cx).value().to_string();
                    view.reset_gem_rows();
                    cx.notify();
                }
            }),
        ];

        let mut view = Self {
            leagues: Vec::new(),
            selected_league: None,
            league_menu_open: false,
            markets: Vec::new(),
            price_overview: None,
            price_error: None,
            gem_overview: None,
            gem_error: None,
            show_prices: true,
            show_gems: false,
            gem_strategy: pages::gem_flipping::GemStrategy::Level,
            gem_time_range: pages::time_range::TimeRangeSelection::default(),
            gem_sort_column: pages::gem_flipping::GemSortColumn::Net,
            gem_sort_ascending: false,
            gem_search: String::new(),
            include_low_confidence_gems: true,
            gem_search_input,
            gem_table_scroll: pages::components::TableScroll::default(),
            gem_row_limit: pages::components::RowLimit::default(),
            lens_price_input,
            _input_subscriptions: input_subscriptions,
            use_facetors_lens: false,
            lens_price_text: String::new(),
            lens_count: 1,
            error: None,
        };
        view.load_dashboard(cx);
        view
    }

    fn reset_gem_rows(&mut self) {
        self.gem_row_limit.reset();
        self.gem_table_scroll.reset();
    }

    fn load_dashboard(&mut self, cx: &mut Context<Self>) {
        self.reset_gem_rows();
        let preferred_league = self
            .selected_league
            .as_ref()
            .map(|league| league.id.clone());
        #[cfg(not(target_arch = "wasm32"))]
        let task = http_runtime().spawn(fetch_dashboard(preferred_league));

        cx.spawn(async move |this, cx| {
            #[cfg(not(target_arch = "wasm32"))]
            let result = match task.await {
                Ok(result) => result,
                Err(error) => Err(error.to_string()),
            };
            #[cfg(target_arch = "wasm32")]
            let result = fetch_dashboard(preferred_league).await;

            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok((
                        leagues,
                        selected_league,
                        markets,
                        price_overview,
                        price_error,
                        gem_overview,
                        gem_error,
                    )) => {
                        view.leagues = leagues;
                        view.selected_league = selected_league;
                        view.markets = markets;
                        view.price_overview = price_overview;
                        view.price_error = price_error;
                        view.gem_overview = gem_overview;
                        view.gem_error = gem_error;
                        view.league_menu_open = false;
                        view.error = None;
                    }
                    Err(error) => {
                        view.leagues.clear();
                        view.selected_league = None;
                        view.markets.clear();
                        view.price_overview = None;
                        view.price_error = None;
                        view.gem_overview = None;
                        view.gem_error = None;
                        view.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_league(&mut self, league: LeagueOption, cx: &mut Context<Self>) {
        self.selected_league = Some(league);
        self.league_menu_open = false;
        self.load_dashboard(cx);
    }
}

#[derive(Serialize)]
struct SelectLeagueRequest<'a> {
    id: &'a str,
}

async fn fetch_dashboard(
    preferred_league: Option<String>,
) -> Result<
    (
        Vec<LeagueOption>,
        Option<LeagueOption>,
        Vec<ExchangeMarketSnapshot>,
        Option<CurrentExchangePriceOverview>,
        Option<String>,
        Option<GemMarketOverview>,
        Option<String>,
    ),
    String,
> {
    let client = reqwest::Client::new();
    let leagues_response = client
        .get(service_url("/api/v1/leagues")?)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !leagues_response.status().is_success() {
        return Err(format!(
            "Service returned HTTP {}",
            leagues_response.status()
        ));
    }
    let leagues = leagues_response
        .json::<Vec<LeagueOption>>()
        .await
        .map_err(|error| error.to_string())?;

    let status_response = client
        .get(service_url("/api/v1/status")?)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !status_response.status().is_success() {
        return Err(format!(
            "Service returned HTTP {}",
            status_response.status()
        ));
    }
    let status = status_response
        .json::<ServiceStatus>()
        .await
        .map_err(|error| error.to_string())?;

    let selected_id = preferred_league
        .filter(|id| leagues.iter().any(|league| &league.id == id))
        .or_else(|| {
            status
                .selected_league
                .clone()
                .filter(|id| leagues.iter().any(|league| &league.id == id))
        })
        .or_else(|| leagues.first().map(|league| league.id.clone()));
    let selected_league = selected_id
        .as_ref()
        .and_then(|id| leagues.iter().find(|league| &league.id == id))
        .cloned();

    if let Some(league_id) = selected_id.as_deref()
        && status.selected_league.as_deref() != Some(league_id)
    {
        let selection_response = client
            .post(service_url("/api/v1/selection")?)
            .json(&SelectLeagueRequest { id: league_id })
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !selection_response.status().is_success() {
            return Err(format!(
                "Service returned HTTP {}",
                selection_response.status()
            ));
        }
    }

    let mut status = status;
    status.selected_league = selected_id.clone();
    let markets_url = reqwest::Url::parse_with_params(
        &service_url("/api/v1/exchange/markets")?,
        selected_id
            .as_deref()
            .map(|league_id| vec![("league", league_id), ("limit", "100")])
            .unwrap_or_else(|| vec![("limit", "100")]),
    )
    .map_err(|error| error.to_string())?;
    let markets_response = client
        .get(markets_url)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !markets_response.status().is_success() {
        return Err(format!(
            "Service returned HTTP {}",
            markets_response.status()
        ));
    }
    let markets = markets_response
        .json::<Vec<ExchangeMarketSnapshot>>()
        .await
        .map_err(|error| error.to_string())?;

    let (price_overview, price_error) = if let Some(league_id) = selected_id.as_deref() {
        let prices_url = reqwest::Url::parse_with_params(
            &service_url("/api/v1/exchange/prices")?,
            [("league", league_id), ("type", "Currency")],
        )
        .map_err(|error| error.to_string())?;
        match client.get(prices_url).send().await {
            Ok(response) if response.status().is_success() => {
                match response.json::<CurrentExchangePriceOverview>().await {
                    Ok(overview) => (Some(overview), None),
                    Err(error) => (None, Some(error.to_string())),
                }
            }
            Ok(response) => (
                None,
                Some(format!("Price feed returned HTTP {}", response.status())),
            ),
            Err(error) => (None, Some(error.to_string())),
        }
    } else {
        (None, None)
    };

    let (gem_overview, gem_error) = if let Some(league_id) = selected_id.as_deref() {
        let gems_url =
            reqwest::Url::parse_with_params(&service_url("/api/v1/gems")?, [("league", league_id)])
                .map_err(|error| error.to_string())?;
        match client.get(gems_url).send().await {
            Ok(response) if response.status().is_success() => {
                match response.json::<GemMarketOverview>().await {
                    Ok(overview) => (Some(overview), None),
                    Err(error) => (None, Some(error.to_string())),
                }
            }
            Ok(response) => (
                None,
                Some(format!("Gem market returned HTTP {}", response.status())),
            ),
            Err(error) => (None, Some(error.to_string())),
        }
    } else {
        (None, None)
    };

    Ok((
        leagues,
        selected_league,
        markets,
        price_overview,
        price_error,
        gem_overview,
        gem_error,
    ))
}

impl Render for MarketView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_league_name = self
            .selected_league
            .as_ref()
            .map(|league| league.name.clone())
            .unwrap_or_else(|| "Loading leagues...".to_owned());

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x10191b))
            .text_color(rgb(0xe5ecea))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(0x334347))
                    .px_4()
                    .py_2()
                    .child(
                        div()
                            .id("league-picker-toggle")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.league_menu_open = !view.league_menu_open;
                                cx.notify();
                            }))
                            .px_3()
                            .py_2()
                            .border_1()
                            .border_color(rgb(0x526568))
                            .child(format!("League: {selected_league_name}  v")),
                    )
                    .child(
                        div()
                            .id("refresh-status")
                            .on_click(cx.listener(|view, _, _, cx| view.load_dashboard(cx)))
                            .px_3()
                            .py_2()
                            .border_1()
                            .border_color(rgb(0x526568))
                            .child("Refresh"),
                    ),
            )
            .when(self.league_menu_open, |root| {
                root.child(
                    div()
                        .flex()
                        .flex_col()
                        .border_b_1()
                        .border_color(rgb(0x334347))
                        .px_6()
                        .py_2()
                        .children(self.leagues.iter().cloned().map(|league| {
                            let selected_league = league.clone();
                            div()
                                .id(format!("league-option-{}", league.id))
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.select_league(selected_league.clone(), cx);
                                }))
                                .px_3()
                                .py_2()
                                .child(league.name)
                        })),
                )
            })
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .px_4()
                    .py_2()
                    .gap_2()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .border_b_1()
                            .border_color(rgb(0x334347))
                            .pb_1()
                            .child(
                                div()
                                    .id("current-prices-tab")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.show_prices = true;
                                        view.show_gems = false;
                                        cx.notify();
                                    }))
                                    .px_3()
                                    .py_2()
                                    .text_sm()
                                    .text_color(if self.show_prices {
                                        rgb(0xe5ecea)
                                    } else {
                                        rgb(0xa6b8b5)
                                    })
                                    .child("Current prices"),
                            )
                            .child(
                                div()
                                    .id("gem-flipping-tab")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.show_prices = false;
                                        view.show_gems = true;
                                        cx.notify();
                                    }))
                                    .px_3()
                                    .py_2()
                                    .text_sm()
                                    .text_color(if self.show_gems {
                                        rgb(0xe5ecea)
                                    } else {
                                        rgb(0xa6b8b5)
                                    })
                                    .child("Gem flipping"),
                            )
                            .child(
                                div()
                                    .id("ggg-history-tab")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.show_prices = false;
                                        view.show_gems = false;
                                        cx.notify();
                                    }))
                                    .px_3()
                                    .py_2()
                                    .text_sm()
                                    .text_color(if self.show_prices || self.show_gems {
                                        rgb(0xa6b8b5)
                                    } else {
                                        rgb(0xe5ecea)
                                    })
                                    .child("GGG history"),
                            ),
                    )
                    .when(self.show_prices, |main| {
                        main.child(pages::current_prices::render(self))
                    })
                    .when(self.show_gems, |main| {
                        main.child(pages::gem_flipping::render(self, cx))
                    })
                    .when(!self.show_prices && !self.show_gems, |main| {
                        main.child(pages::ggg_history::render(self))
                    }),
            )
    }
}

fn open_market_window(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1080.), px(720.)), cx);
    gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        },
        cx,
        |window, cx| cx.new(|cx| MarketView::new(window, cx)),
    )
    .expect("failed to open market window");
}

fn short_item_id(value: &str) -> &str {
    value.rsplit('/').next().unwrap_or(value)
}

fn format_side_values(values: &std::collections::BTreeMap<String, u64>) -> String {
    values
        .iter()
        .map(|(item, amount)| format!("{} {amount}", short_item_id(item)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn service_url(path: &str) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let window = web_sys::window().ok_or_else(|| "Browser window is unavailable".to_owned())?;
        let origin = window
            .location()
            .origin()
            .map_err(|error| format!("Unable to read browser origin: {error:?}"))?;
        Ok(format!("{origin}{path}"))
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Ok(format!("http://127.0.0.1:8787{path}"))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_desktop() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        open_market_window(cx);
    });
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn run_web() {
    console_error_panic_hook::set_once();
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        open_market_window(cx);
    });
}

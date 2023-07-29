use crate::error_template::{AppError, ErrorTemplate};
use cfg_if::cfg_if;
use leptos::*;
use leptos_meta::*;
use leptos_router::*;
use serde::{Deserialize, Serialize};

cfg_if! {
    if #[cfg(feature = "ssr")] {
        use libsql::Database;

        pub async fn db() -> Result<(), ServerFnError> {
            // .headers on
            // .mode column
            let db = Database::open("file:?mode=memory&cache=shared");
            Ok(())
        }

        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub struct Todo {
            id: u16,
            title: String,
            completed: bool,
        }
    } else {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub struct Todo {
            id: u16,
            title: String,
            completed: bool,
        }
    }
}

#[component]
pub fn App(cx: Scope) -> impl IntoView {
    // Provides context that manages stylesheets, titles, meta tags, etc.
    provide_meta_context(cx);

    view! {
        cx,

        // injects a stylesheet into the document <head>
        // id=leptos means cargo-leptos will hot-reload this stylesheet
        <Stylesheet id="leptos" href="/pkg/start-axum.css"/>

        // sets the document title
        <Title text="Welcome to Leptos"/>

        // content for this welcome page
        <Router fallback=|cx| {
            let mut outside_errors = Errors::default();
            outside_errors.insert_with_default_key(AppError::NotFound);
            view! { cx,
                <ErrorTemplate outside_errors/>
            }
            .into_view(cx)
        }>
            <main>
                <Routes>
                    <Route path="" view=|cx| view! { cx, <HomePage/> }/>
                </Routes>
            </main>
        </Router>
    }
}

/// Renders the home page of your application.
#[component]
fn HomePage(cx: Scope) -> impl IntoView {
    // Creates a reactive value to update the button
    let (count, set_count) = create_signal(cx, 0);
    let on_click = move |_| {
        spawn_local(async {
            let _ = get_todo().await;
        });
        set_count.update(|count| *count += 1);
    };

    view! { cx,
        <h1>"Welcome to Leptos!"</h1>
        <button on:click=on_click>"Click Me: " {count}</button>
    }
}

#[server(GetTodos, "/api")]
pub async fn get_todo() -> Result<(), ServerFnError> {
    let conn = db();

    Ok(())
}

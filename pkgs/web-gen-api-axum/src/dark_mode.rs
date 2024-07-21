use leptos::*;
use leptos_meta::Meta;
use leptos_router::ActionForm;

#[server(ToggleDarkMode, "/api")]
pub async fn toggle_dark_mode(cx: Scope, prefers_dark: bool) -> Result<bool, ServerFnError> {
    use hyper::header::{HeaderValue, SET_COOKIE};
    use hyper::HeaderMap;

    let mut headers = HeaderMap::new();

    headers.insert(
        SET_COOKIE,
        HeaderValue::from_str(&format!("darkmode={}", prefers_dark))
            .expect("to create header value"),
    );

    std::thread::sleep(std::time::Duration::from_millis(5000));

    Ok(prefers_dark)
}

#[cfg(not(feature = "ssr"))]
fn initial_prefers_dark(_cx: Scope) -> bool {
    use wasm_bindgen::JsCast;
    use web_sys::HtmlDocument;

    let window = web_sys::window().expect("no global `window` exists");
    let doc = window
        .document()
        .expect("should have a document")
        .unchecked_into::<HtmlDocument>();
    let cookie = doc.cookie().unwrap_or_default();
    cookie.contains("darkmode=true")
}

#[cfg(feature = "ssr")]
fn initial_prefers_dark(cx: Scope) -> bool {
    use hyper::header::COOKIE;
    use leptos_axum::RequestParts;

    use_context::<RequestParts>(cx)
        .and_then(|req| {
            req.headers.get(COOKIE).and_then(|cookie| {
                std::str::from_utf8(cookie.as_bytes())
                    .ok()
                    .map(|cookies| cookies.contains("darkmode=true"))
            })
        })
        .unwrap_or(true)
}

#[component]
pub fn DarkModeToggle(cx: Scope) -> impl IntoView {
    let initial = initial_prefers_dark(cx);

    let toggle_dark_mode_action = create_server_action::<ToggleDarkMode>(cx);
    // input is `Some(value)` when pending, and `None` if not pending
    let input = toggle_dark_mode_action.input();
    // value contains most recently-returned value
    let value = toggle_dark_mode_action.value();

    // NOTE: if you're following along the with video, this was implemented
    // incorrectly at the time I made it, due to a bug in <ActionForm/> that
    // was not resetting input. This is how it should have been implemented
    // all along, which would also have fixed the bug at 49:24!
    let prefers_dark = move || {
        match (input(), value()) {
            // if there's some current input, use that optimistically
            (Some(submission), _) => submission.prefers_dark,
            // otherwise, if there was a previous value confirmed by server, use that
            (_, Some(Ok(value))) => value,
            // otherwise, use the initial value
            _ => initial,
        }
    };

    let color_scheme = move || {
        if prefers_dark() {
            "dark".to_string()
        } else {
            "light".to_string()
        }
    };

    view! { cx,
        <Meta
            name="color-scheme"
            content=color_scheme
        />
        <ActionForm action=toggle_dark_mode_action>
            <input
                type="hidden"
                name="prefers_dark"
                value=move || (!prefers_dark()).to_string()
            />
            <input
                type="submit"
                value=move || {
                    if prefers_dark() {
                        "Switch to Light Mode"
                    } else {
                        "Switch to Dark Mode"
                    }
                }
            />
        </ActionForm>
    }
}

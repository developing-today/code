use leptos::*;

fn main() {
    leptos::mount_to_body(|cx| view! { cx, <App/> })
}

#[component]
fn App(cx: Scope) -> impl IntoView {
    let (count, set_count) = create_signal(cx, 0);

    view! { cx,
        <button
            on:click=move |_| {
                if (count() == 0) {
                    set_count(42);
                } else {
                    set_count(count() + 1);
                }
            }
        >
            "Click me: "
            {move || count.get()}
        </button>
    }
}

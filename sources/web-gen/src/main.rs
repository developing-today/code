use leptos::*;

fn main() {
    leptos::mount_to_body(|cx| view! { cx, <App/> })
}

#[component]
fn App(cx: Scope) -> impl IntoView {
    let default = 0;
    let (count, set_count) = create_signal(cx, default);

    view! { cx,
        <button
            class="bg-white hover:bg-gray-100 font-semibold py-2 px-4 border border-gray-400 rounded shadow"
            class=("text-grey-800", move || count() == 0)
            class=("text-blue-800", move || count() != 0 && count() % 2 == 0)
            class=("text-green-800", move || count() % 2 != 0)
            on:click=move |_| {
                set_count.update(|n| *n += if *n == default { 42 } else { 1 });
            }
        >
            "Click me: "
            {count}
        </button>
    }
}

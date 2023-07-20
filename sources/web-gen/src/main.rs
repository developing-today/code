use leptos::*;

fn main() {
    leptos::mount_to_body(|cx| view! { cx, <App/> })
}

#[component]
fn App(cx: Scope) -> impl IntoView {
    let (count, set_count) = create_signal(cx, 0);

    view! { cx,
        <button class="bg-white hover:bg-gray-100 text-gray-800 font-semibold py-2 px-4 border border-gray-400 rounded shadow"
            on:click=move |_| {
                set_count.update(|n| *n += if *n == 0 { 42 } else { 1 });
            }
        >
            "Click me: "
            {count}
        </button>
    }
}

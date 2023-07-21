use leptos::*;

fn main() {
    leptos::mount_to_body(|cx| view! { cx, <App/> })
}

#[component]
fn App(cx: Scope) -> impl IntoView {
    let default = 0;
    let (count, set_count) = create_signal(cx, default);
    let double_count = move || count() * 2;

    let values = vec![0, 1, 2];
    let length = 3;
    let counters = (1..=length).map(|idx| create_signal(cx, idx));

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
        <br />
        <ProgressBar value=count/>
        <br />
        <ProgressBar value=Signal::derive(cx, double_count)/>
        <br />
        <p>
            "Double Count: "
            {double_count}
        </p>
        <br />
        <p>{values.clone()}</p>
        <br />
        <ul>
            {values.clone().into_iter()
                .map(|n| view! { cx, <li>{n}</li>})
                .collect::<Vec<_>>()}
        </ul>
        <br />
        <ul>
            {values.clone().into_iter()
                .map(|n| view! { cx, <li>{n}</li>})
                .collect_view(cx)}
        </ul>
        <br />
        <ul>
            {counters
                .map(|(count, set_count)| {
                    view! { cx,
                        <li>
                            <button
                                on:click=move |_| set_count.update(|n| *n += 1)
                            >
                                {count}
                            </button>
                        </li>
                    }
                })
                .collect_view(cx)}
        </ul>
    }
}

#[component]
fn ProgressBar(
    cx: Scope,
    #[prop(default = 100)]
    max: u16,
    #[prop(into)] // #[prop(optional)]
    value: Signal<i32>
) -> impl IntoView
{
    view! { cx,
        <progress
            max=max
            value=value
        />
    }
}

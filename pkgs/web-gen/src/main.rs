use color_eyre::eyre::Result;
use js_sys::try_iter;
use leptos::ev::Event;
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::*;
use leptos_router::components::Form;
use leptos_router::hooks::use_query_map;
use std::str::FromStr;
use tracing::info;
use wasm_bindgen::JsCast;
use web_gen_core::{generate_divisible_i_from_n_up_to_m, generate_random_number_and_multiply};
use web_sys::HtmlInputElement;

// wasm/leptos doesn't run these things right.. need multiple builds?
// need to get back to buck2 build setups i think, but cargo might support
// build and serve wasm from one and build and serve apiserver from another?
// cargo add axum tokio poem-openapi
fn main() {
    // color_eyre::install().unwrap();
    // info!("Starting app");
    leptos::mount::mount_to_body(App)
}

#[component]
fn App() -> impl IntoView {
    view! { <TestApp/> }
}

#[component]
fn TestApp() -> impl IntoView {
    let default = 0;
    let (count, set_count) = signal(default);
    let double_count = move || count.get() * 2;

    let values = vec![1, 2, 4, 8, 16, 32, 49, 51];
    let coins = vec![1, 5, 7, 12, 20, 50];
    let length = 3;
    let counters = (1..=length).map(|idx| signal(idx));

    view! {
        <h1>"Hello, World!"</h1>
        <button
            class="bg-white hover:bg-gray-100 font-semibold py-2 px-4 border border-gray-400 rounded shadow"
            class=("text-grey-800", move || count.get() == 0)
            class=("text-blue-800", move || count.get() != 0 && count.get() % 2 == 0)
            class=("text-green-800", move || count.get() % 2 != 0)
            on:click=move |_| {
                set_count.update(|n| *n += if *n == default { 42 } else { 1 });
            }
        >
            "Click me: "
            {move || count.get()}
        </button>
        <br />
        <ProgressBar value=count/>
        <br />
        <ProgressBar value=Signal::derive(double_count)/>
        <br />
        <p>
            "Double Count: "
            {double_count}
        </p>
        <br />
        <p>{values.clone()}</p>
        <br />
        <ul>
            values: {values.clone().into_iter()
                .map(|n| view! { <li>{n}</li>})
                .collect::<Vec<_>>()}
        </ul>
        <br />
        <ul>
            values*random: {values.clone().into_iter()
                .map(|n| view! { <li>{generate_random_number_and_multiply(n)}</li>})
                .collect_view()}
        </ul>
        <br />
        <ul>
            coins: {coins.clone().into_iter()
                .map(|n| view! { <li>{n}</li>})
                .collect::<Vec<_>>()}
        </ul>
        <br />
        <ul>
            coins*random: {coins.clone().into_iter()
                .map(|n| view! { <li>{generate_random_number_and_multiply(n)}</li>})
                .collect_view()}
        </ul>
        <br />
        <ul>
            coins_values_to_100: {coins.clone().into_iter()
                .map(|n| view! { <li>{generate_divisible_i_from_n_up_to_m(n, n, 100)}</li>})
                .collect_view()}
        </ul>
        <br />
        <ul>
            {counters
                .map(|(count, set_count)| {
                    view! {
                        <li>
                            <button
                                on:click=move |_| set_count.update(|n| *n += 1)
                            >
                                {move || count.get()}
                            </button>
                        </li>
                    }
                })
                .collect_view()}
        </ul>
        // <br />
        // <DetailedFormExample/>
        <br />
        <RandomMultiplierForm/>
    }
}

#[component]
fn ProgressBar(
    #[prop(default = 100)] max: u16,
    #[prop(into)] // #[prop(optional)]
    value: Signal<i32>,
) -> impl IntoView {
    view! {
        <progress
            max=max
            value=move || value.get()
        />
    }
}

#[component]
pub fn RandomMultiplierForm() -> impl IntoView {
    let (result, set_result) = signal("".to_string());

    let submit = move |event: SubmitEvent| {
        event.prevent_default();

        if let Some(form) = event
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlFormElement>().ok())
        {
            let elements = form.elements();
            let length = elements.length();

            for index in 0..length {
                if let Some(element) = elements.item(index) {
                    if let Some(input) = element.dyn_into::<web_sys::HtmlInputElement>().ok() {
                        if input.name() == "number" {
                            if let Ok(num) = i32::from_str(&input.value()) {
                                let result_val = generate_random_number_and_multiply(num);
                                set_result.set(format!("Result: {}", result_val));
                                break;
                            } else {
                                set_result.set("Invalid input".to_string());
                                break;
                            }
                        }
                    }
                }
            }
        } else {
            set_result.set("Invalid event".to_string());
        }
    };

    view! {
        <form on:submit=submit>
            Number: <input type="number" name="number"/>
            <br />
            <input type="submit"/>
        </form>
        <br />
        <div>{result}</div>
    }
}

// #[component]
// pub fn FormExample(cx: Scope) -> impl IntoView {
//     let (input_value, set_input_value) = signal(String::new());
//     let (result, set_result) = signal(String::new());

//     create_effect(cx, move |_| {
//         let value = input_value().clone();
//         let new_result = generate_random_number_and_multiply_result(parse_input(&value));
//         set_result(match new_result {
//             Ok(num) => num,
//             Err(e) => e.to_string(),
//         });
//     });

//     view! {
//         <Form method="GET" action="">
//             <input type="search" name="search" value=input_value()
//                 oninput=move |event: Event| {
//                     if let Some(target) = event.target() {
//                         if let Ok(input) = target.dyn_into::<HtmlInputElement>() {
//                             set_input_value(input.value());
//                         }
//                     }
//                 } />
//             <div> { "Result: ".to_owned() + &result() } </div>
//         </Form>
//     }
// }

#[component]
pub fn DetailedFormExample() -> impl IntoView {
    // reactive access to URL query
    let query = use_query_map();
    let name = move || query.get().get("name").unwrap_or_default();
    let number = move || query.get().get("number").unwrap_or_default();
    let select = move || query.get().get("select").unwrap_or_default();

    view! {
        // read out the URL query strings
        <table>
            <tr>
                <td><code>"name"</code></td>
                <td>{name}</td>
            </tr>
            <tr>
                <td><code>"number"</code></td>
                <td>{number}</td>
            </tr>
            <tr>
                <td><code>"select"</code></td>
                <td>{select}</td>
            </tr>
        </table>
        // <Form/> will navigate whenever submitted
        <h2>"Manual Submission"</h2>
        <Form method="GET" action="">
            // input names determine query string key
            <input type="text" name="name" value=name/>
            <input type="number" name="number" value=number/>
            <select name="select">
                // `selected` will set which starts as selected
                <option selected=move || select() == "A">
                    "A"
                </option>
                <option selected=move || select() == "B">
                    "B"
                </option>
                <option selected=move || select() == "C">
                    "C"
                </option>
            </select>
            // submitting should cause a client-side
            // navigation, not a full reload
            <input type="submit"/>
        </Form>
        // This <Form/> uses some JavaScript to submit
        // on every input
        <h2>"Automatic Submission"</h2>
        <Form method="GET" action="">
            <input
                type="text"
                name="name"
                value=name
                // this oninput attribute will cause the
                // form to submit on every input to the field
                oninput="this.form.requestSubmit()"
            />
            <input
                type="number"
                name="number"
                value=number
                oninput="this.form.requestSubmit()"
            />
            <select name="select"
                onchange="this.form.requestSubmit()"
            >
                <option selected=move || select() == "A">
                    "A"
                </option>
                <option selected=move || select() == "B">
                    "B"
                </option>
                <option selected=move || select() == "C">
                    "C"
                </option>
            </select>
            // submitting should cause a client-side
            // navigation, not a full reload
            <input type="submit"/>
        </Form>
    }
}

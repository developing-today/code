
use leptos::prelude::*;
use leptos::*;
use leptos_router::*;
use rand::Rng;
use std::str::FromStr;
use futures::executor::block_on;
use futures::future::ready;
use color_eyre::eyre::Result;
use tracing::info;
use leptos::ev::Event;
use wasm_bindgen::JsCast;
use web_sys::HtmlInputElement;
use leptos::ev::SubmitEvent;

// wasm/leptos doesn't run these things right.. need multiple builds?
// need to get back to buck2 build setups i think, but cargo might support
// build and serve wasm from one and build and serve apiserver from another?
// cargo add axum tokio poem-openapi
fn main() {
    // color_eyre::install().unwrap();
    // info!("Starting app");
    leptos::mount_to_body(|cx| view! { cx, <App/> })
}

#[component]
fn App(cx: Scope) -> impl IntoView {
    view! { cx, <TestApp/> }
}

#[component]
fn TestApp(cx: Scope) -> impl IntoView {
    let default = 0;
    let (count, set_count) = create_signal(cx, default);
    let double_count = move || count() * 2;

    let values = vec![0, 1, 2];
    let length = 3;
    let counters = (1..=length).map(|idx| create_signal(cx, idx));

    view! { cx,
        <h1>"Hello, World!"</h1>
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
        // <br />
        // <DetailedFormExample/>
        <br />
        <RandomMultiplierForm/>
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

fn generate_random_number() -> i32 {
    let mut rng = rand::thread_rng();
    rng.gen_range(0..101)
}

fn generate_random_number_and_multiply(input: i32) -> i32 {
    generate_random_number() * input
}

fn parse_input(input: &str) -> Result<i32, std::num::ParseIntError> {
    i32::from_str(input)
}

fn generate_random_number_and_multiply_result(input: Result<i32, std::num::ParseIntError>) -> Result<String, std::num::ParseIntError> {
    match input {
        Ok(num) => {
            let result = generate_random_number() * num;
            Ok(result.to_string())
        },
        Err(e) => Err(e),
    }
}

fn generate_random_number_and_multiply_str_future(input: String) -> impl std::future::Future<Output = Result<String, String>> {
    let input_number = parse_input(&input);
    let result = generate_random_number_and_multiply_result(input_number);
    ready(match result {
        Ok(num) => Ok(num),
        Err(e) => Err(e.to_string()),
    })
}

#[component]
pub fn RandomMultiplierForm(cx: Scope) -> impl IntoView {
    let (result, set_result) = create_signal(cx, "".to_string());

    let submit = move |event: SubmitEvent| {
        event.prevent_default();
        if let Some(input_element) = event.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()) {
            if let Ok(num) = i32::from_str(&input_element.value()) {
                let result_val = generate_random_number_and_multiply(num);
                set_result(format!("Result: {}", result_val));
            } else {
                set_result("Invalid input value".to_string());
            }
        } else {
            set_result("Invalid input event".to_string());
        }
    };

    view! { cx,
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
//     let (input_value, set_input_value) = create_signal(cx, String::new());
//     let (result, set_result) = create_signal(cx, String::new());

//     create_effect(cx, move |_| {
//         let value = input_value().clone();
//         let new_result = generate_random_number_and_multiply_result(parse_input(&value));
//         set_result(match new_result {
//             Ok(num) => num,
//             Err(e) => e.to_string(),
//         });
//     });

//     view! { cx,
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
pub fn DetailedFormExample(cx: Scope) -> impl IntoView {
    // reactive access to URL query
    let query = use_query_map(cx);
    let name = move || query().get("name").cloned().unwrap_or_default();
    let number = move || query().get("number").cloned().unwrap_or_default();
    let select = move || query().get("select").cloned().unwrap_or_default();

    view! { cx,
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

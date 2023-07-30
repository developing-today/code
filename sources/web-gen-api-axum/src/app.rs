use crate::error_template::{AppError, ErrorTemplate};
use cfg_if::cfg_if;
use leptos::{html::Input, *};
use leptos_meta::*;
use leptos_router::*;

cfg_if! {
    if #[cfg(feature = "ssr")] {
        use std::sync::{Mutex};
        use once_cell::sync::Lazy;
        use libsql::{Connection, Database};

        fn conn() -> Connection {
            connect(None)
        }

        fn connect(conn_str: Option<&str>) -> Connection {
            database(conn_str).connect().unwrap()
        }

        fn db() -> Database {
            database(None)
        }

        fn database(conn_str: Option<&str>) -> Database {
            open_db(conn_str.unwrap_or("🙂:memory"))
        }

        fn open_db(name: &str) -> Database {
            match Database::open(
                match name {
                    "🙂:memory" => "file:file?mode=memory&cache=shared",
                    _ => name,
                }
            ) {
                Ok(db) => db,
                Err(e) => {
                    println!("Failed to connect to DB: {}", e);
                    panic!("Failed to connect to DB: {}", e);
                }
            }
        }

        fn connect_to_db(db: &Database) -> Connection {
            println!("Connecting to DB");
            db.connect().unwrap()
        }

        fn log_hello_world_with_conn(conn: &Connection) {
            let hello_rows = conn.execute("SELECT 'hello, world!'", ()).unwrap().unwrap();
            let hello_row = hello_rows.next().unwrap().unwrap();
            log!("{}", hello_row.get::<&str>(0).unwrap());
        }

        fn create_visitors_table_with_conn(conn: &Connection) {
            conn.execute(
                "CREATE TABLE visitors (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                datetime DEFAULT CURRENT_TIMESTAMP,
                data TEXT);
                ", ()).unwrap();
        }

        fn insert_visitor_data_with_conn(conn: &Connection, data: &str) {
            let params = vec![libsql::Value::from(data)];
            log!("Inserting visitor data: {}", data);
            conn.execute("INSERT INTO visitors (data) VALUES (?);", params).unwrap();
        }

        fn insert_visitor_data(data: Option<&str>) {
            insert_visitor_data_with_conn(&conn(), data.unwrap_or(""));
        }

        pub fn insert_visitor() {
            insert_visitor_data(None);
        }

        fn insert_initial_visitor_data_with_conn(conn: &Connection) {
            insert_visitor_data_with_conn(conn, "_start");
        }

        fn get_rows_count_with_conn(conn: &Connection, table: &str) -> i32 {
            let count_rows = conn.execute(&format!("SELECT COUNT(*) FROM {};", table), ()).unwrap().unwrap();
            let count_row = count_rows.next().unwrap().unwrap();
            let rows_count: i32 = count_row.get::<i32>(0).unwrap();
            // log!("Inserted {} rows", rows_count);
            // table: rows:
            log!("table: {}, row_count: {}", table, rows_count);
            rows_count
        }

        pub fn get_rows_count(table: &str) -> i32 {
            get_rows_count_with_conn(&conn(), table)
        }

        fn log_all_visitors_with_conn(conn: &Connection) {
            let select_rows = conn.execute("SELECT * FROM visitors;", ()).unwrap().unwrap();
            while let Some(row) = select_rows.next().unwrap() {
                log!("id: {}, datetime: {}, data: {}", row.get::<i32>(0).unwrap(), row.get::<&str>(1).unwrap(), row.get::<&str>(2).unwrap());
            }
        }

        pub fn log_all_visitors() {
            log_all_visitors_with_conn(&conn());
        }

        pub static DB: Lazy<Mutex<Database>> = Lazy::new(|| {
            Mutex::new(db())
        });

        pub static CONNECTION: Lazy<Mutex<Connection>> = Lazy::new(|| {
            let db = DB.lock().unwrap();
            let conn = connect_to_db(&*db);
            Mutex::new(conn)
        });

        pub async fn initialize_static_db() {
            log!("Initializing DB");

            // Accessing CONNECTION will trigger its initialization
            // In-memory DBs are initialized on first access
            // are shared between threads and persist
            // until the last open connection is closed
            let conn = CONNECTION.lock().unwrap();

            log_hello_world_with_conn(&*conn);
            create_visitors_table_with_conn(&*conn);
            insert_initial_visitor_data_with_conn(&*conn);
            get_rows_count_with_conn(&*conn, "visitors");
            log_all_visitors_with_conn(&*conn);

            log!("Initialized DB");
        }
    }
}

#[server(AddVisitorData, "/api")]
pub async fn add_visitor(data: Option<String>) -> Result<u32, ServerFnError> {
    log!("Hello, AddVisitorData!");
    insert_visitor_data(data.as_deref());
    let rows_count = get_rows_count("visitors");
    Ok(rows_count as u32)
}

#[component]
fn HomePage(cx: Scope) -> impl IntoView {
    let add_visitor_action = create_server_action::<AddVisitorData>(cx);
    let value_signal = add_visitor_action.value();

    // Using derived signal for the count
    let count = create_memo(cx, move |_| value_signal().unwrap_or(Ok(0)).unwrap_or(0));
    let submission_message = create_memo(cx, move |_| {
        if count.get() > 0 {
            format!("Server has received {} submissions total!", count.get())
        } else {
            "You have not submitted yet!".to_string()
        }
    });

    view! { cx,
        <h1>"Welcome to Leptos!"</h1>

        <ActionForm action=add_visitor_action>
            <label>
                "What do you need to do?"
                <input type="text" name="data"/>
            </label>
            <input type="submit" value="Submit"/>
        </ActionForm>

        <p>{submission_message}</p>
        <p>"Current submission count: " {count}</p>
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

pub fn SimpleCounter(cx: Scope) -> impl IntoView {
    let (value, set_value) = create_signal(cx, 0);

    let clear = move |_ev| set_value.set(0);
    let decrement = move |_ev| set_value.update(|value| *value -= 1);
    let increment = move |_ev| set_value.update(|value| *value += 1);

    view! {
        cx,
        <div>
            <button on:click=clear>"Clear"</button>
            <button on:click=decrement>"-1"</button>
            <span>"Value: " {move || value.get().to_string()} "!"</span>
            <button on:click=increment>"+1"</button>
        </div>
    }
}

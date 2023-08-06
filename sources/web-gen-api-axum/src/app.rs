use crate::dark_mode::DarkModeToggle;
use crate::error_template::{AppError, ErrorTemplate};
use cfg_if::cfg_if;
use leptos::*;
use leptos_meta::*;
use leptos_router::*;

cfg_if! {
    if #[cfg(feature = "ssr")] {
        use std::sync::{Mutex};
        use once_cell::sync::Lazy;
        use libsql::{Connection, Database};

        // Enums
        enum Mode {
            Memory(MemoryMode),
            File,
        }

        enum JournalType {
            Wal,
            Wal2,
        }

        enum SyncMode {
            Off,
            Normal,
            Full,
            Extra,
        }

        // Structs
        struct MemoryMode {
            shared: bool,
        }

        impl MemoryMode {
            fn new() -> Self {
                Self { shared: true } // Default to shared memory, always per-process only
            }
        }

        struct DatabaseConfig {
            path: Option<String>,
            mode: Mode,
        }

        impl DatabaseConfig {
            fn new(path: Option<String>, mode: Option<Mode>) -> Self {
                Self {
                    path,
                    mode: mode.unwrap_or(Mode::Memory(MemoryMode::new())),
                }
            }
        }

        struct JournalMode {
            journal_type: JournalType,
            journal_size_limit: usize,
            checkpoint_size_limit: usize,
            sync: SyncMode,
        }

        struct ConnectionConfig {
            options: ConnectionOptions,
            pragma: ConnectionPragma,
        }

        struct ConnectionOptions {
            random_rowid: bool,
        }

        struct ConnectionPragma {
            busy_timeout: usize,
            journal_mode: Option<JournalMode>,
            prepragma: Vec<String>,
            postpragma: Vec<String>,
            preexec: Vec<String>,
            postexec: Vec<String>,
        }

        impl ConnectionConfig {
            fn new(options: ConnectionOptions, pragma: ConnectionPragma) -> Self {
                Self { options, pragma }
            }
        }

        struct MyDatabase {
            database: Database,
            config: MyDatabaseConfig,
        }

        struct MyDatabaseConfig {
            database: DatabaseConfig,
            connection: Option<ConnectionConfig>,
        }

        struct MyConnection {
            connection: Connection,
            config: Option<ConnectionConfig>,
        }

        impl MyConnection {
            fn new(connection: Connection, config: Option<ConnectionConfig>) -> Self {
                Self { connection, config }
            }
        }

        pub struct MyDatabaseMutex {
            database: Mutex<Database>,
            config: MyDatabaseConfig,
        }

        impl From<MyDatabase> for MyDatabaseMutex {
            fn from(db: MyDatabase) -> Self {
                Self {
                    database: Mutex::new(db.database),
                    config: db.config,
                }
            }
        }

        pub struct MyConnectionMutex {
            connection: Mutex<Connection>,
            config: Option<ConnectionConfig>,
        }

        impl From<MyConnection> for MyConnectionMutex {
            fn from(conn: MyConnection) -> Self {
                Self {
                    connection: Mutex::new(conn.connection),
                    config: conn.config,
                }
            }
        }

        // Static variables
        //https://cj.rs/blog/sqlite-pragma-cheatsheet-for-performance-and-consistency/
        pub static MY_DB: Lazy<MyDatabaseMutex> = Lazy::new(|| MyDatabaseMutex {
            database: Mutex::new(my_db(None, None).database),
            config: my_db(None, None).config,
        });

        pub static MY_CONNECTION: Lazy<MyConnectionMutex> = Lazy::new(|| MyConnectionMutex {
            connection: Mutex::new(my_conn(None, None).connection),
            config: my_conn(None, None).config,
        });


        // Connection Functions
        fn my_database(database_config: Option<DatabaseConfig>, connection_config: Option<ConnectionConfig>) -> MyDatabase {
            open_db(database_config, connection_config)
        }

        fn my_db(database_config: Option<DatabaseConfig>, connection_config: Option<ConnectionConfig>) -> MyDatabase {
            my_database(database_config, connection_config)
        }

        fn my_connection(database_config: Option<DatabaseConfig>, connection_config: Option<ConnectionConfig>) -> MyConnection {
            let db = my_db(database_config, None);
            let conn = db.database.connect().unwrap();
            MyConnection::new(conn, connection_config)
        }

        fn my_conn(database_config: Option<DatabaseConfig>, connection_config: Option<ConnectionConfig>) -> MyConnection {
            my_connection(database_config, connection_config)
        }

        fn conn() -> Connection {
            connect(None, None)
        }

        fn connect(database_config: Option<DatabaseConfig>, connection_config: Option<ConnectionConfig>) -> Connection {
            let db = open_db(database_config, connection_config);
            db.database.connect().unwrap()
        }

        fn db() -> Database {
            database(None, None)
        }

        fn database(database_config: Option<DatabaseConfig>, connection_config: Option<ConnectionConfig>) -> Database {
            open_db(database_config, connection_config).database
        }

        fn build_conn_string(database_config: &DatabaseConfig) -> String {
            if let Some(path) = &database_config.path {
                if path.contains("://") {
                    return path.clone();
                }
            }

            match &database_config.mode {
                Mode::Memory(memory_mode) => {
                    if let Some(path) = &database_config.path {
                        if memory_mode.shared {
                            format!("file:{}?mode=memory&cache=shared", path)
                        } else {
                            format!("file:{}?mode=memory", path)
                        }
                    } else if memory_mode.shared {
                        "file::memory:?cache=shared".to_string()
                    } else {
                        ":memory:".to_string()
                    }
                }
                Mode::File => {
                    if let Some(path) = &database_config.path {
                        format!("file:{}", path)
                    } else {
                        panic!("File mode requires a path");
                    }
                }
            }
        }

        fn open_db(database_config: Option<DatabaseConfig>, connection_config: Option<ConnectionConfig>) -> MyDatabase {
            let db_config = database_config.unwrap_or_else(|| DatabaseConfig::new(None, None));

            match Database::open(&build_conn_string(&db_config)) {
                Ok(db) => {
                    MyDatabase {
                        database: db,
                        config: MyDatabaseConfig {
                            database: db_config,
                            connection: connection_config,
                        },
                    }
                }
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

        // Log and Table Operations
        fn log_hello_world_with_conn(conn: &Connection) {
            let hello_rows = conn.execute("SELECT 'hello, world!'", ()).unwrap().unwrap();
            let hello_row = hello_rows.next().unwrap().unwrap();
            log!("{}", hello_row.get::<&str>(0).unwrap());
        }

        fn create_visitors_table_with_conn(conn: &Connection) {
            conn.execute(
                "CREATE TABLE visitors (
                datetime DEFAULT CURRENT_TIMESTAMP,
                data TEXT) RANDOM ROWID;
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
            log!("table: {}, row_count: {}", table, rows_count);
            rows_count
        }

        pub fn get_rows_count(table: &str) -> i32 {
            get_rows_count_with_conn(&conn(), table)
        }

        fn log_all_visitors_with_conn(conn: &Connection) {
            let select_rows = conn.execute("SELECT rowid, * FROM visitors;", ()).unwrap().unwrap();
            while let Some(row) = select_rows.next().unwrap() {
                log!("id: {}, datetime: {}, data: {}", row.get::<i32>(0).unwrap(), row.get::<&str>(1).unwrap(), row.get::<&str>(2).unwrap());
            }
        }

        pub fn log_all_visitors() {
            log_all_visitors_with_conn(&conn());
        }

        pub async fn initialize_static_db() {
            log!("Initializing DB");
            let conn = MY_CONNECTION.connection.lock().unwrap();
            log_hello_world_with_conn(&*conn);
            create_visitors_table_with_conn(&*conn);
            insert_initial_visitor_data_with_conn(&*conn);
            get_rows_count_with_conn(&*conn, "visitors");
            log_all_visitors_with_conn(&*conn);
            log!("Initialized DB");
        }
    }
}

// Server Functions
#[server(AddVisitorData, "/api")]
pub async fn add_visitor(data: Option<String>) -> Result<(), ServerFnError> {
    log!("Hello, AddVisitorData!");
    insert_visitor_data(data.as_deref());
    Ok(())
}

#[server(GetVisitorRows, "/api")]
pub async fn get_visitor_rows(_unused: Option<String>) -> Result<u32, ServerFnError> {
    log!("Fetching visitor count!");
    log_all_visitors();
    Ok(get_rows_count("visitors") as u32)
}

#[component]
fn HomePage(cx: Scope) -> impl IntoView {
    // Creating a resource to fetch the visitor count from the server
    let visitor_rows_resource = create_resource(
        cx,
        || (),
        |_| async move { get_visitor_rows(None).await.unwrap_or(0) },
    );

    // Triggering an initial call to fetch the count
    visitor_rows_resource.refetch();

    // Using derived signal for the count
    let count = create_memo(cx, move |_| visitor_rows_resource.read(cx).unwrap_or(0));
    let add_visitor_action = create_server_action::<AddVisitorData>(cx);
    let (user_count, set_user_count) = create_signal(cx, 0);

    // Trigger a refetch of visitor_rows_resource whenever add_visitor_action has a new value
    create_effect(cx, move |_: Option<()>| {
        if add_visitor_action.value().get().is_some() {
            visitor_rows_resource.refetch();
            set_user_count.update(|count| *count += 1); // Increment user count
        }
        () // Return unit type
    });

    let submission_message = create_memo(cx, move |_| {
        if user_count.get() > 0 {
            format!(
                "You have submitted {} time{}!",
                user_count.get(),
                if user_count.get() == 1 { "" } else { "s" }
            )
        } else {
            "You have not submitted yet.".to_string()
        }
    });
    let submission_count_message = create_memo(cx, move |_| {
        if count.get() < 2 {
            "".to_string()
        } else {
            format!("Total submissions: {}", count.get())
        }
    });

    view! { cx,
        <h1>"Welcome to Leptos!"</h1>
        <DarkModeToggle/>
        <ActionForm action=add_visitor_action>
            <label>
                "What data to submit?"<br /><br />
                <input type="text" name="data"/>
            </label>
            <input type="submit" value="Submit"/>
        </ActionForm><br />

        <p>{submission_message}</p>
        <p>{submission_count_message}</p>
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

#[component]
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

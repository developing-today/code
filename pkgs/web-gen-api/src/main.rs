#[cfg(feature = "ssr")]
#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // use libsql::Database;

    // let db = Database::open(":memory:");
    // let conn = db.connect().unwrap();

    // conn.execute("CREATE TABLE IF NOT EXISTS users (email TEXT)", ()) .unwrap();
    // conn.execute("INSERT INTO users (email) VALUES ('alice@example.org')", ()).unwrap();

    use actix_files::Files;
    use actix_web::*;
    use leptos::prelude::*;
    use leptos_actix::{generate_route_list, LeptosRoutes};
    use leptos_meta::MetaTags;
    use leptos_start::app::*;

    let conf = leptos::config::get_configuration(None).unwrap();
    let addr = conf.leptos_options.site_addr;
    // Generate the list of routes in your Leptos App
    let routes = generate_route_list(|| view! { <App/> });

    HttpServer::new(move || {
        let leptos_options = conf.leptos_options.clone();
        let site_root = leptos_options.site_root.clone();
        let route_leptos_options = leptos_options.clone();

        App::new()
            .route("/api/{tail:.*}", leptos_actix::handle_server_fns())
            // serve JS/WASM/CSS from `pkg`
            .service(Files::new("/pkg", format!("{site_root}/pkg")))
            // serve other assets from the `assets` directory
            .service(Files::new("/assets", site_root.to_string()))
            // serve the favicon from /favicon.ico
            .service(favicon)
            .leptos_routes(routes.to_owned(), move || {
                view! {
                    <!DOCTYPE html>
                    <html lang="en">
                        <head>
                            <meta charset="utf-8"/>
                            <meta name="viewport" content="width=device-width, initial-scale=1"/>
                            <AutoReload options=route_leptos_options.clone() />
                            <HydrationScripts options=route_leptos_options.clone() />
                            <MetaTags/>
                        </head>
                        <body>
                            <App/>
                        </body>
                    </html>
                }
            })
            .app_data(web::Data::new(leptos_options))
        //.wrap(middleware::Compress::default())
    })
    .bind(&addr)?
    .run()
    .await
}

#[cfg(feature = "ssr")]
#[actix_web::get("favicon.ico")]
async fn favicon(
    leptos_options: actix_web::web::Data<leptos::config::LeptosOptions>,
) -> actix_web::Result<actix_files::NamedFile> {
    let leptos_options = leptos_options.into_inner();
    let site_root = &leptos_options.site_root;
    Ok(actix_files::NamedFile::open(format!(
        "{site_root}/favicon.ico"
    ))?)
}

#[cfg(not(any(feature = "ssr", feature = "csr")))]
pub fn main() {
    // no client-side main function
    // unless we want this to work with e.g., Trunk for pure client-side testing
    // see lib.rs for hydration function instead
    // see optional feature `csr` instead
}

#[cfg(all(not(feature = "ssr"), feature = "csr"))]
pub fn main() {
    // a client-side main function is required for using `trunk serve`
    // prefer using `cargo leptos serve` instead
    // to run: `trunk serve --open --features csr`
    use leptos::*;
    use leptos_start::app::*;
    use wasm_bindgen::prelude::wasm_bindgen;

    console_error_panic_hook::set_once();

    leptos::mount_to_body(move || {
        // note: for testing it may be preferrable to replace this with a
        // more specific component, although leptos_router should still work
        view! { <App/> }
    });
}

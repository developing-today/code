use idp_provider::{app_from_config, Config};

#[tokio::main]
async fn main() {
    let config = Config::from_env();
    let bind = config.bind.clone();
    let issuer = config.issuer.clone();
    let app = app_from_config(config).unwrap_or_else(|e| {
        eprintln!("idp-provider: startup failed: {e}");
        std::process::exit(1);
    });
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {bind}: {e}"));
    println!("idp-provider listening on {bind} (issuer {issuer})");
    axum::serve(listener, app).await.expect("server error");
}

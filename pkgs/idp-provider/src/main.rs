use idp_provider::{app_from_config, Config};

#[tokio::main]
async fn main() {
    // Operator helper (on-host only): hash a local-door credential
    // for the provider registry config. The credential is read from
    // the IDP_LOCAL_CREDENTIAL environment variable; only the
    // PBKDF2 hash is printed — the plaintext is never written
    // anywhere by this tool.
    if std::env::args().nth(1).as_deref() == Some("hash-credential") {
        match std::env::var("IDP_LOCAL_CREDENTIAL") {
            Ok(cred) if !cred.is_empty() => {
                println!("{}", idp_provider::registry::hash_credential(&cred));
            }
            _ => {
                eprintln!("idp-provider hash-credential: IDP_LOCAL_CREDENTIAL is not set");
                std::process::exit(2);
            }
        }
        return;
    }

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

use idp_provider::{app_from_config, Config};
use std::sync::Arc;

/// Operator identity-store commands (#13415): link / unlink / links.
///
/// These run on-host against the same store the server uses. The
/// registry config path comes from `IDP_PROVIDER_CONFIG`; the store
/// path from `IDP_PROVIDER_IDENTITY_DB` or the config's
/// `identity_db_path`. Every change is written to the identity
/// history by the store and mirrored into the audit record, exactly
/// like a door-login link. Run them while the server is stopped or
/// accept that the server's in-memory view is its own connection —
/// SQLite serializes the writes either way.
fn identity_cli(args: &[String]) -> i32 {
    let cfg_path = match std::env::var("IDP_PROVIDER_CONFIG") {
        Ok(p) => std::path::PathBuf::from(p),
        Err(_) => {
            eprintln!(
                "idp-provider {cmd}: IDP_PROVIDER_CONFIG is not set",
                cmd = args[0]
            );
            return 2;
        }
    };
    let registry = match idp_provider::registry::ProviderConfig::load(&cfg_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("idp-provider: {e}");
            return 1;
        }
    };
    let identity_path = std::env::var("IDP_PROVIDER_IDENTITY_DB")
        .map(std::path::PathBuf::from)
        .ok()
        .or_else(|| registry.identity_db_path.clone());
    let store = match idp_provider::identities::IdentityStore::open(identity_path.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("idp-provider: {e}");
            return 1;
        }
    };
    // Seed first so the CLI sees the same rows the server seeded.
    let _ = store.seed(&registry.account.account_id, &registry.account.identities);
    let audit_path = std::env::var("IDP_PROVIDER_AUDIT_DB")
        .map(std::path::PathBuf::from)
        .ok()
        .or_else(|| registry.audit_db_path.clone());
    let audit = idp_provider::audit::AuditLog::open(audit_path.as_deref())
        .map(Arc::new)
        .ok();

    match args[0].as_str() {
        "links" => match store.list() {
            Ok(rows) => {
                for r in rows {
                    println!(
                        "{}:{} account={} status={} linked_at={} unlinked_at={}",
                        r.provider,
                        r.subject,
                        r.account_id,
                        r.status,
                        r.linked_at,
                        r.unlinked_at
                            .map(|t| t.to_string())
                            .unwrap_or_else(|| "-".into())
                    );
                }
                0
            }
            Err(e) => {
                eprintln!("idp-provider: {e}");
                1
            }
        },
        "link" | "unlink" => {
            let (provider, subject) = match (args.get(1), args.get(2)) {
                (Some(p), Some(s)) => (p.clone(), s.clone()),
                _ => {
                    eprintln!("usage: idp-provider {} <provider> <subject>", args[0]);
                    return 2;
                }
            };
            let identity_key = format!("{provider}:{subject}");
            if args[0] == "link" {
                match store.link(
                    &provider,
                    &subject,
                    &registry.account.account_id,
                    "operator-cli",
                ) {
                    Ok(event) => {
                        if event != "none" {
                            if let Some(a) = &audit {
                                let _ = a.record("link", "allowed", event, "", &identity_key);
                            }
                        }
                        println!("{identity_key}: {event}");
                        0
                    }
                    Err(e) => {
                        eprintln!("idp-provider: {e}");
                        1
                    }
                }
            } else {
                match store.unlink(&provider, &subject, "operator-cli") {
                    Ok(true) => {
                        if let Some(a) = &audit {
                            let _ = a.record("unlink", "allowed", "unlink", "", &identity_key);
                        }
                        println!("{identity_key}: unlinked");
                        0
                    }
                    Ok(false) => {
                        println!("{identity_key}: not active (nothing changed)");
                        0
                    }
                    Err(e) => {
                        eprintln!("idp-provider: {e}");
                        1
                    }
                }
            }
        }
        _ => 2,
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Operator helper (on-host only): hash a local-door credential
    // for the provider registry config. The credential is read from
    // the IDP_LOCAL_CREDENTIAL environment variable; only the
    // PBKDF2 hash is printed — the plaintext is never written
    // anywhere by this tool.
    if args.first().map(|s| s.as_str()) == Some("hash-credential") {
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
    // Operator identity-store commands (#13415).
    if matches!(
        args.first().map(|s| s.as_str()),
        Some("link") | Some("unlink") | Some("links")
    ) {
        std::process::exit(identity_cli(&args));
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

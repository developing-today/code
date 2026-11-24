//! The `fleet orgs` domain (developing-today/code#8884): a registry of
//! GitHub organisations, the fleet Apps and their installations, kept
//! in Turso and regenerated from each App's own JWT.
//!
//! Coverage is never read through an OAuth token: `GET /app` and
//! `GET /app/installations` answer for the App itself, paginated by
//! their `Link` headers, so a sync sees every installation.

pub mod cli;
pub mod github;
pub mod jwt;
pub mod link;
pub mod list;
pub mod schema;
pub mod sql;
pub mod sync;
pub mod turso;

pub use cli::run;

//! The storage seam: one statement at a time, with JSON-typed
//! arguments and rows. Turso answers over HTTP ([`crate::turso`]);
//! tests answer from SQLite.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Rows {
    pub rows: Vec<Vec<Value>>,
}

pub trait Sql {
    fn execute(&self, sql: &str, args: &[Value]) -> Result<Rows, String>;
}

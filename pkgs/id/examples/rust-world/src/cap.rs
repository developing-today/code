//! The request side of the capability protocol, mirroring `RocCap.roc`:
//! a session reserves `r0`, `r1`, … ids, records each request's
//! capability, builds the request documents `wants` re-announces after
//! replay, and routes host result events back to the capability that
//! asked. See `examples/roc-world/ABI.md` for the wire protocol.

use crate::json::{quote, Value};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CapSession {
    next: u64,
    pending: Vec<(String, String)>, // (id, cap)
}

impl CapSession {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild a session from a snapshot's pending list, like
    /// `RocCap.from_pending`: ids continue after the highest `r<n>`.
    pub fn from_pending(pending: Vec<(String, String)>) -> Self {
        let next = pending
            .iter()
            .filter_map(|(id, _)| id.strip_prefix('r'))
            .filter_map(|n| n.parse::<u64>().ok())
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);
        CapSession { next, pending }
    }

    pub fn pending(&self) -> &[(String, String)] {
        &self.pending
    }

    /// Reserve the next request id and record its capability, like
    /// `RocCap.issue`. Returns the id (`r0`, `r1`, …).
    pub fn issue(&mut self, cap: &str) -> String {
        let id = format!("r{}", self.next);
        self.next += 1;
        self.pending.push((id.clone(), cap.to_string()));
        id
    }

    /// A request document for `wants`: `{"id":..,"cap":..}` plus an
    /// `args` object when args JSON is supplied.
    pub fn request_doc(&self, id: &str, cap: &str, args_json: Option<&str>) -> String {
        match args_json {
            Some(args) => format!(
                "{{\"id\":{},\"cap\":{},\"args\":{}}}",
                quote(id),
                quote(cap),
                args
            ),
            None => format!("{{\"id\":{},\"cap\":{}}}", quote(id), quote(cap)),
        }
    }

    pub fn now_request(&mut self) -> String {
        let id = self.issue("time.now");
        self.request_doc(&id, "time.now", None)
    }

    pub fn players_request(&mut self) -> String {
        let id = self.issue("players.list");
        self.request_doc(&id, "players.list", None)
    }

    pub fn world_request(&mut self) -> String {
        let id = self.issue("world.info");
        self.request_doc(&id, "world.info", None)
    }

    pub fn chat_request(&mut self, text: &str) -> String {
        let id = self.issue("chat.say");
        let args = format!("{{\"text\":{}}}", quote(text));
        self.request_doc(&id, "chat.say", Some(&args))
    }

    pub fn history_request(&mut self, count: Option<u64>) -> String {
        let id = self.issue("chat.history");
        let args = count.map(|c| format!("{{\"count\":{c}}}"));
        self.request_doc(&id, "chat.history", args.as_deref())
    }

    pub fn records_request(&mut self, prefix: Option<&str>, after: Option<&str>) -> String {
        let id = self.issue("records.read");
        let mut fields = Vec::new();
        if let Some(p) = prefix {
            fields.push(format!("\"prefix\":{}", quote(p)));
        }
        if let Some(a) = after {
            fields.push(format!("\"after\":{}", quote(a)));
        }
        let args = format!("{{{}}}", fields.join(","));
        self.request_doc(&id, "records.read", Some(&args))
    }

    pub fn schedule_at_request(&mut self, schedule_id: &str, at_ms: i64) -> String {
        let id = self.issue("schedule.at");
        let args = format!("{{\"id\":{},\"at\":{}}}", quote(schedule_id), at_ms);
        self.request_doc(&id, "schedule.at", Some(&args))
    }

    pub fn schedule_cancel_request(&mut self, schedule_id: &str) -> String {
        let id = self.issue("schedule.cancel");
        let args = format!("{{\"id\":{}}}", quote(schedule_id));
        self.request_doc(&id, "schedule.cancel", Some(&args))
    }

    /// Route a host result event (`{"cap":"result","id":..,...}`) back
    /// to the capability that asked, like `RocCap.answered`. Returns the
    /// cap name for a known pending id.
    pub fn answered(&self, result: &Value) -> Option<String> {
        if result.get("cap").and_then(|c| c.as_str()) != Some("result") {
            return None;
        }
        let id = result.get("id").and_then(|i| i.as_str())?;
        self.pending
            .iter()
            .find(|(pid, _)| pid == id)
            .map(|(_, cap)| cap.clone())
    }

    /// Retire a pending request, like `RocCap.done`.
    pub fn done(&mut self, id: &str) {
        self.pending.retain(|(pid, _)| pid != id);
    }

    /// The `wants` document: `{"v":1,"subscribe":[...],"requests":[...]}`
    /// with empty parts omitted, as the ABI allows.
    pub fn wants_doc(&self, subscribe: &[&str], request_docs: &[String]) -> String {
        let mut parts = vec!["\"v\":1".to_string()];
        if !subscribe.is_empty() {
            let subs = subscribe
                .iter()
                .map(|s| quote(s))
                .collect::<Vec<_>>()
                .join(",");
            parts.push(format!("\"subscribe\":[{subs}]"));
        }
        if !request_docs.is_empty() {
            parts.push(format!("\"requests\":[{}]", request_docs.join(",")));
        }
        format!("{{{}}}", parts.join(","))
    }
}

/// Rows of a `chat.history` result value: the `lines` array's objects,
/// like `RocCap.history_lines`.
pub fn history_lines(result_value: &Value) -> Vec<&Value> {
    result_value
        .get("lines")
        .and_then(|l| l.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::decode;

    #[test]
    fn session_issues_routes_and_retires() {
        let mut s = CapSession::new();
        let doc = s.now_request();
        assert_eq!(doc, "{\"id\":\"r0\",\"cap\":\"time.now\"}");
        let doc2 = s.chat_request("hi \"there\"");
        assert_eq!(
            doc2,
            "{\"id\":\"r1\",\"cap\":\"chat.say\",\"args\":{\"text\":\"hi \\\"there\\\"\"}}"
        );
        let result = decode("{\"cap\":\"result\",\"id\":\"r1\",\"ok\":true,\"value\":{}}").unwrap();
        assert_eq!(s.answered(&result), Some("chat.say".to_string()));
        s.done("r1");
        assert_eq!(s.answered(&result), None);
        let wants = s.wants_doc(&["players"], &[doc]);
        assert_eq!(
            wants,
            "{\"v\":1,\"subscribe\":[\"players\"],\"requests\":[{\"id\":\"r0\",\"cap\":\"time.now\"}]}"
        );
        let mut rebuilt = CapSession::from_pending(s.pending().to_vec());
        assert_eq!(rebuilt.issue("world.info"), "r1");
    }
}

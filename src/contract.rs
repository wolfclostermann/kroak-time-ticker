//! Contract-driven request server for kroak-time.
//!
//! What the ticker serves to kroak-time isn't hard-coded: it's described by a
//! kroak-time request-source definition file (kroak-time's
//! src/request_source/ schema — see contracts/kroak-time-ticker-request-source.toml),
//! and the very same file is what kroak-time polls with. The ticker reads it
//! "backwards":
//!
//! - `[config].base_url` is the address to listen on.
//! - Each operation's `method` + `path` + `body` template identifies an
//!   incoming call. Template values without placeholders (e.g.
//!   `"command":"getSerial"`) must match the incoming body exactly; a value
//!   that is exactly one placeholder (`{request_id}`, `{accepting}`) binds that
//!   value from the incoming body; any other placeholder (`"{api_key}"`) is
//!   accepted as-is. When several operations match, the one with the most
//!   literal values wins.
//! - `extract` paths say where to put things in the response: the serial at
//!   `poll_guard.extract.value`, the request array at `fetch.extract.items`,
//!   and each semantic field at its `fetch.extract.fields` path.

use crate::requests::{RequestStore, SongRequest};
use anyhow::{bail, Context, Result};
use axum::{body::Bytes, extract::State, http::{Method, Uri}, Json, Router};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

pub const BUILTIN_CONTRACT: &str = include_str!("contracts/kroak-time-ticker-request-source.toml");

/// kroak-time's semantic request fields (its `def::SEMANTIC_FIELDS`).
const SEMANTIC_FIELDS: &[&str] = &[
    "request_id",
    "singer",
    "artist",
    "title",
    "key_change",
    "request_time",
    "requested_by_first_name",
    "requested_by_last_name",
];

#[derive(Deserialize)]
struct ContractDoc {
    schema_version: u32,
    #[serde(default)]
    config: BTreeMap<String, toml::Value>,
    operations: Operations,
}

#[derive(Deserialize)]
struct Operations {
    poll_guard: Option<OpDoc>,
    fetch: Option<OpDoc>,
    delete: Option<OpDoc>,
    set_accepting: Option<OpDoc>,
}

#[derive(Deserialize)]
struct OpDoc {
    #[serde(default = "default_method")]
    method: String,
    #[serde(default = "default_path")]
    path: String,
    body: Option<String>,
    extract: Option<ExtractDoc>,
}

#[derive(Deserialize)]
struct ExtractDoc {
    value: Option<String>,
    items: Option<String>,
    fields: Option<BTreeMap<String, String>>,
}

fn default_method() -> String {
    "POST".into()
}
fn default_path() -> String {
    "/".into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpKind {
    PollGuard,
    Fetch,
    Delete,
    SetAccepting,
}

/// One node of a body template, with placeholders resolved to roles.
#[derive(Debug, Clone, PartialEq)]
enum Pattern {
    /// Must equal the incoming value.
    Literal(Value),
    /// Binds the incoming value to this placeholder name.
    Bind(String),
    /// Contains placeholders that aren't a whole value — accepted as-is.
    Any,
    Object(Vec<(String, Pattern)>),
}

#[derive(Debug)]
struct Op {
    kind: OpKind,
    method: Method,
    path: String,
    /// `None` for a body-less operation.
    body: Option<Pattern>,
    /// Response path: serial (poll_guard) or items array (fetch).
    out_path: Option<Vec<String>>,
    /// fetch only: semantic field -> path within each item.
    fields: Vec<(String, Vec<String>)>,
}

#[derive(Debug)]
pub struct Contract {
    pub listen: SocketAddr,
    ops: Vec<Op>,
    requested_by_first: String,
    requested_by_last: String,
}

/// Make sure `path` exists, writing the built-in contract there if not —
/// same idea as kroak-time seeding its request-sources folder.
pub fn ensure_contract_file(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::write(path, BUILTIN_CONTRACT)
            .with_context(|| format!("writing default request contract to {}", path.display()))?;
        tracing::info!("Wrote default request contract to {}", path.display());
    }
    Ok(())
}

pub fn load(path: &Path) -> Result<Contract> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse(&text).with_context(|| format!("in {}", path.display()))
}

pub fn parse(text: &str) -> Result<Contract> {
    let doc: ContractDoc = toml::from_str(text)?;
    if doc.schema_version != 1 {
        bail!("unsupported schema_version {}", doc.schema_version);
    }
    let cfg_str = |k: &str| doc.config.get(k).and_then(|v| v.as_str()).map(str::to_string);

    let base_url = cfg_str("base_url").context("[config] must contain a string `base_url`")?;
    let url = url::Url::parse(&base_url).context("config.base_url")?;
    let host = url.host_str().context("config.base_url has no host")?;
    let host = if host == "localhost" { "127.0.0.1" } else { host };
    let port = url.port_or_known_default().context("config.base_url has no port")?;
    let listen: SocketAddr = format!("{}:{port}", host.trim_matches(['[', ']']))
        .parse()
        .with_context(|| format!("config.base_url host {host:?} must be an IP address or localhost"))?;
    let base_path = url.path().trim_end_matches('/').to_string();

    let mut ops = Vec::new();
    for (kind, doc_op) in [
        (OpKind::PollGuard, doc.operations.poll_guard),
        (OpKind::Fetch, doc.operations.fetch),
        (OpKind::Delete, doc.operations.delete),
        (OpKind::SetAccepting, doc.operations.set_accepting),
    ] {
        let Some(o) = doc_op else { continue };
        let name = format!("{kind:?}");
        let method = match o.method.as_str() {
            "GET" => Method::GET,
            "POST" => Method::POST,
            m => bail!("operations.{name}: method must be GET or POST, got {m:?}"),
        };
        let body = o
            .body
            .as_deref()
            .map(parse_template)
            .transpose()
            .with_context(|| format!("operations.{name}.body"))?;
        let extract = o.extract.as_ref();
        let out_path = match kind {
            OpKind::PollGuard => Some(extract.and_then(|e| e.value.as_deref()).context("operations.poll_guard needs extract.value")?),
            OpKind::Fetch => Some(extract.and_then(|e| e.items.as_deref()).context("operations.fetch needs extract.items")?),
            _ => None,
        }
        .map(parse_path)
        .transpose()?;
        let mut fields = Vec::new();
        if kind == OpKind::Fetch {
            let map = extract.and_then(|e| e.fields.as_ref()).context("operations.fetch needs extract.fields")?;
            for (semantic, p) in map {
                if !SEMANTIC_FIELDS.contains(&semantic.as_str()) {
                    bail!("operations.fetch.extract.fields: unknown field {semantic:?} (known: {})", SEMANTIC_FIELDS.join(", "));
                }
                fields.push((semantic.clone(), parse_path(p)?));
            }
            for required in ["request_id", "singer", "artist", "title"] {
                if !map.contains_key(required) {
                    bail!("operations.fetch.extract.fields is missing required field {required:?}");
                }
            }
        }
        let path = format!("{base_path}/{}", o.path.trim_start_matches('/'));
        ops.push(Op { kind, method, path, body, out_path, fields });
    }
    if !ops.iter().any(|o| o.kind == OpKind::Fetch) {
        bail!("contract needs operations.fetch");
    }

    Ok(Contract {
        listen,
        ops,
        requested_by_first: cfg_str("requested_by_first_name").unwrap_or_else(|| "Phone".into()),
        requested_by_last: cfg_str("requested_by_last_name").unwrap_or_else(|| "{device_code}".into()),
    })
}

/// Response paths are plain dot-separated keys (`data.requests`). kroak-time
/// also allows `[n]` indexes when *reading*, but there's no sensible way to
/// *build* a response from one, so they're rejected here.
fn parse_path(p: &str) -> Result<Vec<String>> {
    if p.contains('[') {
        bail!("response path {p:?}: array indexes aren't supported on the serving side");
    }
    let parts: Vec<String> = p.split('.').map(str::to_string).collect();
    if parts.iter().any(|s| s.is_empty()) {
        bail!("invalid path {p:?}");
    }
    Ok(parts)
}

// Printable (raw control characters aren't legal inside JSON strings) and
// implausible in a real template.
const PH_OPEN: &str = "@@ph:";
const PH_CLOSE: &str = "@@";

/// Turns a kroak-time body template into a [`Pattern`]. Placeholders follow
/// kroak-time's template rules — `{identifier}`, everything else literal,
/// `{{` escapes — and are swapped for sentinel strings (quoted when they sit
/// outside a JSON string) so the result parses as JSON.
fn parse_template(t: &str) -> Result<Pattern> {
    let mut out = String::with_capacity(t.len() + 16);
    let mut in_str = false;
    let mut chars = t.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' if in_str => {
                out.push(c);
                if let Some((_, n)) = chars.next() {
                    out.push(n);
                }
            }
            '"' => {
                in_str = !in_str;
                out.push(c);
            }
            '{' => {
                let rest = &t[i + 1..];
                if rest.starts_with('{') {
                    out.push('{');
                    chars.next();
                    continue;
                }
                let len = rest
                    .char_indices()
                    .take_while(|&(j, ch)| if j == 0 { ch.is_ascii_alphabetic() || ch == '_' } else { ch.is_ascii_alphanumeric() || ch == '_' })
                    .count();
                if len > 0 && rest[len..].starts_with('}') {
                    let name = &rest[..len];
                    if in_str {
                        out.push_str(&format!("{PH_OPEN}{name}{PH_CLOSE}"));
                    } else {
                        out.push_str(&format!("\"{PH_OPEN}{name}{PH_CLOSE}\""));
                    }
                    for _ in 0..=len {
                        chars.next();
                    }
                } else {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    let v: Value = serde_json::from_str(&out).context("template doesn't form valid JSON")?;
    Ok(to_pattern(v))
}

fn to_pattern(v: Value) -> Pattern {
    match v {
        Value::Object(m) => Pattern::Object(m.into_iter().map(|(k, v)| (k, to_pattern(v))).collect()),
        Value::String(s) if s.contains(PH_OPEN) => {
            let inner = s.strip_prefix(PH_OPEN).and_then(|r| r.strip_suffix(PH_CLOSE));
            match inner {
                Some(name) if !name.contains(PH_CLOSE) => Pattern::Bind(name.to_string()),
                _ => Pattern::Any,
            }
        }
        other => Pattern::Literal(other),
    }
}

/// Number of literal values matched, or `None` on mismatch.
fn match_pattern(p: &Pattern, v: &Value, binds: &mut BTreeMap<String, Value>) -> Option<usize> {
    match p {
        Pattern::Literal(l) => (l == v).then_some(1),
        Pattern::Bind(name) => {
            binds.insert(name.clone(), v.clone());
            Some(0)
        }
        Pattern::Any => Some(0),
        Pattern::Object(fields) => {
            let obj = v.as_object()?;
            let mut score = 0;
            for (k, sub) in fields {
                score += match_pattern(sub, obj.get(k)?, binds)?;
            }
            Some(score)
        }
    }
}

/// The literal parts of a pattern, echoed back in the response (so e.g.
/// `{"command":"getSerial"}` answers with `"command":"getSerial"` like a real
/// OpenKJ server does).
fn literals(p: &Pattern) -> Value {
    match p {
        Pattern::Object(fields) => {
            let mut m = Map::new();
            for (k, sub) in fields {
                match sub {
                    Pattern::Literal(l) => {
                        m.insert(k.clone(), l.clone());
                    }
                    Pattern::Object(_) => {
                        m.insert(k.clone(), literals(sub));
                    }
                    _ => {}
                }
            }
            Value::Object(m)
        }
        _ => Value::Object(Map::new()),
    }
}

fn set_path(root: &mut Value, path: &[String], value: Value) {
    let mut cur = root;
    for (i, key) in path.iter().enumerate() {
        if !cur.is_object() {
            *cur = Value::Object(Map::new());
        }
        let obj = cur.as_object_mut().unwrap();
        if i == path.len() - 1 {
            obj.insert(key.clone(), value);
            return;
        }
        cur = obj.entry(key.clone()).or_insert_with(|| Value::Object(Map::new()));
    }
}

fn fill(template: &str, r: &SongRequest) -> String {
    template
        .replace("{device_code}", &r.device_code())
        .replace("{device_id}", &r.device_id)
        .replace("{fingerprint}", &r.fingerprint)
        .replace("{singer}", &r.singer)
}

impl Contract {
    fn semantic_value(&self, r: &SongRequest, field: &str) -> Value {
        match field {
            "request_id" => r.request_id.into(),
            "singer" => r.singer.clone().into(),
            "artist" => r.artist.clone().into(),
            "title" => r.title.clone().into(),
            "key_change" => r.key_change.into(),
            "request_time" => r.request_time.into(),
            "requested_by_first_name" => fill(&self.requested_by_first, r).into(),
            "requested_by_last_name" => fill(&self.requested_by_last, r).into(),
            _ => Value::Null,
        }
    }

    /// Answers one incoming call, or `None` if no operation matches it.
    fn handle(&self, store: &RequestStore, method: &Method, path: &str, body: &Value) -> Option<Value> {
        let path = if path.is_empty() { "/" } else { path };
        let mut best: Option<(&Op, usize, BTreeMap<String, Value>)> = None;
        for op in self.ops.iter().filter(|o| &o.method == method && o.path == path) {
            let mut binds = BTreeMap::new();
            let score = match &op.body {
                Some(p) => match match_pattern(p, body, &mut binds) {
                    Some(s) => s,
                    None => continue,
                },
                None => 0,
            };
            if best.as_ref().map_or(true, |(_, s, _)| score > *s) {
                best = Some((op, score, binds));
            }
        }
        let (op, _, binds) = best?;
        let mut resp = op.body.as_ref().map(literals).unwrap_or_else(|| Value::Object(Map::new()));

        match op.kind {
            OpKind::PollGuard => set_path(&mut resp, op.out_path.as_ref().unwrap(), store.serial().into()),
            OpKind::Fetch => {
                let items: Vec<Value> = store
                    .list()
                    .iter()
                    .map(|r| {
                        let mut item = Value::Object(Map::new());
                        for (semantic, p) in &op.fields {
                            set_path(&mut item, p, self.semantic_value(r, semantic));
                        }
                        item
                    })
                    .collect();
                set_path(&mut resp, op.out_path.as_ref().unwrap(), Value::Array(items));
            }
            OpKind::Delete => {
                let id = binds.get("request_id").and_then(|v| v.as_i64().or_else(|| v.as_str()?.trim().parse().ok()));
                match id {
                    Some(id) => store.delete(id),
                    None => tracing::warn!("delete call without a usable {{request_id}}: {body}"),
                }
            }
            OpKind::SetAccepting => match binds.get("accepting").and_then(Value::as_bool) {
                Some(v) => store.set_accepting(v),
                None => tracing::warn!("set_accepting call without a boolean {{accepting}}: {body}"),
            },
        }
        Some(resp)
    }
}

/// Serves `contract` for kroak-time to poll. Bodies are parsed by hand rather
/// than with axum's `Json` extractor so a client that omits `Content-Type`
/// still works. Runs until the runtime shuts down (the main server owns the
/// shutdown `Notify`, whose `notify_one` would only wake one listener).
pub async fn run_server(contract: Contract, store: RequestStore) -> Result<()> {
    let listen = contract.listen;
    if !listen.ip().is_loopback() {
        tracing::warn!("Request contract base_url binds {listen}, which isn't loopback — anyone who can reach it can read and delete requests");
    }
    let state = Arc::new((contract, store));
    let app = Router::new()
        .fallback(|State(state): State<Arc<(Contract, RequestStore)>>, method: Method, uri: Uri, body: Bytes| async move {
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let (contract, store) = &*state;
            match contract.handle(store, &method, uri.path(), &body) {
                Some(v) => Json(v),
                None => Json(serde_json::json!({"error": true, "errorString": "no operation in the request contract matches this call"})),
            }
        })
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("binding request server on {listen}"))?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::requests::NewRequest;
    use serde_json::json;

    fn submit(store: &RequestStore, device: &str) -> SongRequest {
        store
            .submit(
                NewRequest {
                    singer: "Alice".into(),
                    artist: "Kings of Leon".into(),
                    title: "Sex on Fire".into(),
                    key_change: -2,
                    device_id: device.into(),
                    fingerprint: "fp".into(),
                    in_catalog: true,
                },
                0,
            )
            .unwrap()
    }

    fn call(c: &Contract, s: &RequestStore, body: Value) -> Value {
        c.handle(s, &Method::POST, "/", &body).expect("an operation should match")
    }

    #[test]
    fn builtin_contract_round_trip() {
        let c = parse(BUILTIN_CONTRACT).unwrap();
        assert_eq!(c.listen, "127.0.0.1:8767".parse().unwrap());
        let s = RequestStore::in_memory();

        let serial0 = call(&c, &s, json!({"command": "getSerial"}))["serial"].as_i64().unwrap();
        let r = submit(&s, "dev-a");
        let guard = call(&c, &s, json!({"command": "getSerial"}));
        assert_eq!(guard["command"], "getSerial");
        assert_ne!(guard["serial"].as_i64().unwrap(), serial0);

        let got = call(&c, &s, json!({"command": "getRequests"}));
        let item = &got["requests"][0];
        assert_eq!(item["request_id"], r.request_id);
        assert_eq!(item["singer"], "Alice");
        assert_eq!(item["key_change"], -2);
        assert_eq!(item["user_first_name"], "Phone");
        assert_eq!(item["user_last_name"], r.device_code());

        call(&c, &s, json!({"command": "deleteRequest", "request_id": r.request_id}));
        assert!(s.list().is_empty());

        call(&c, &s, json!({"command": "setAccepting", "accepting": false}));
        assert!(!s.accepting());
    }

    #[test]
    fn custom_contract_shapes_the_response() {
        let c = parse(
            r#"
            schema_version = 1
            id = "x"
            name = "x"
            mode = "poll"
            [config]
            base_url = "http://localhost:9000/okj/"
            venue_id = 4
            api_key = "k"
            requested_by_first_name = "{singer}'s"
            requested_by_last_name = "phone {device_code}"
            [operations.poll_guard]
            path = "serial"
            body = '{"cmd":"serial","venue_id":{venue_id},"api_key":"{api_key}"}'
            [operations.poll_guard.extract]
            value = "data.serial"
            [operations.fetch]
            path = "list"
            body = '{"cmd":"list","api_key":"{api_key}"}'
            [operations.fetch.extract]
            items = "data.items"
            [operations.fetch.extract.fields]
            request_id = "id"
            singer = "who.name"
            artist = "song.artist"
            title = "song.title"
            requested_by_first_name = "by.first"
            requested_by_last_name = "by.last"
            [operations.delete]
            path = "list"
            body = '{"cmd":"del","id":"{request_id}"}'
            "#,
        )
        .unwrap();
        assert_eq!(c.listen, "127.0.0.1:9000".parse().unwrap());
        let s = RequestStore::in_memory();
        let r = submit(&s, "dev-z");

        // Config placeholders accept whatever the client sends.
        let g = c.handle(&s, &Method::POST, "/okj/serial", &json!({"cmd": "serial", "venue_id": 99, "api_key": "zzz"})).unwrap();
        assert!(g["data"]["serial"].is_number());

        let got = c.handle(&s, &Method::POST, "/okj/list", &json!({"cmd": "list", "api_key": "k"})).unwrap();
        let item = &got["data"]["items"][0];
        assert_eq!(item["who"]["name"], "Alice");
        assert_eq!(item["song"]["title"], "Sex on Fire");
        assert_eq!(item["by"]["first"], "Alice's");
        assert_eq!(item["by"]["last"], format!("phone {}", r.device_code()));

        // Same path, different literal: routed to delete; stringified id accepted.
        c.handle(&s, &Method::POST, "/okj/list", &json!({"cmd": "del", "id": r.request_id.to_string()})).unwrap();
        assert!(s.list().is_empty());

        // Wrong literal / wrong path match nothing.
        assert!(c.handle(&s, &Method::POST, "/okj/list", &json!({"cmd": "nope"})).is_none());
        assert!(c.handle(&s, &Method::POST, "/", &json!({"cmd": "list"})).is_none());
    }

    #[test]
    fn rejects_unknown_semantic_field() {
        let bad = BUILTIN_CONTRACT.replace("request_time = \"request_time\"", "nonsense = \"x\"");
        assert!(parse(&bad).is_err());
    }
}

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    fs, io,
    path::PathBuf,
    time::Duration,
    time::{SystemTime, UNIX_EPOCH},
};

const DEFAULT_MAX_BYTES: usize = 20 * 1024 * 1024;
const DEFAULT_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionOutbound {
    Direct,
    Proxy,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionResult {
    Success,
    Failure(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleAttribution {
    Known(String),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionEvent {
    pub observed_at_unix_ms: u128,
    pub connection_id: u64,
    pub target: String,
    pub port: u16,
    pub outbound: ConnectionOutbound,
    pub rule: RuleAttribution,
    pub result: ConnectionResult,
    pub config_revision: u64,
}

#[derive(Debug, Default)]
struct PendingConnection {
    revision: u64,
    target: Option<(String, u16)>,
    outbound: ConnectionOutbound,
    rule: Option<String>,
}

#[derive(Debug)]
struct ConnectionLogStore {
    path: PathBuf,
    max_bytes: usize,
    retention: Duration,
}

impl ConnectionLogStore {
    fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            max_bytes: DEFAULT_MAX_BYTES,
            retention: DEFAULT_RETENTION,
        }
    }

    fn load(&self) -> io::Result<VecDeque<ConnectionEvent>> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(VecDeque::new()),
            Err(error) => return Err(error),
        };
        Ok(bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .filter_map(|line| serde_json::from_slice(line).ok())
            .map(redact_event)
            .collect())
    }

    fn persist(&self, events: &mut VecDeque<ConnectionEvent>) -> io::Result<()> {
        let cutoff = now_ms().saturating_sub(self.retention.as_millis());
        while events
            .front()
            .is_some_and(|event| event.observed_at_unix_ms < cutoff)
        {
            events.pop_front();
        }
        let mut bytes = serialize_events(events)?;
        while bytes.len() > self.max_bytes && events.pop_front().is_some() {
            bytes = serialize_events(events)?;
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = self.path.with_extension("jsonl.tmp");
        fs::write(&temporary, bytes)?;
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        fs::rename(temporary, &self.path)
    }

    fn clear(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

fn serialize_events(events: &VecDeque<ConnectionEvent>) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for event in events {
        serde_json::to_writer(&mut bytes, event).map_err(io::Error::other)?;
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn redact_event(mut event: ConnectionEvent) -> ConnectionEvent {
    if let ConnectionResult::Failure(detail) = &mut event.result {
        *detail = redact_sensitive(detail);
    }
    event
}

#[derive(Debug, Default)]
pub struct ConnectionLogAdapter {
    revision: u64,
    mappings: BTreeMap<u64, Vec<Option<String>>>,
    pending: BTreeMap<u64, PendingConnection>,
    events: VecDeque<ConnectionEvent>,
    store: Option<ConnectionLogStore>,
    storage_error: Option<String>,
}

impl ConnectionLogAdapter {
    pub fn with_store(path: impl Into<PathBuf>) -> Self {
        let store = ConnectionLogStore::new(path);
        let (mut events, mut storage_error) = match store.load() {
            Ok(events) => (events, None),
            Err(error) => (VecDeque::new(), Some(error.to_string())),
        };
        if storage_error.is_none()
            && let Err(error) = store.persist(&mut events)
        {
            storage_error = Some(error.to_string());
        }
        Self {
            events,
            store: Some(store),
            storage_error,
            ..Self::default()
        }
    }

    pub fn begin_revision(&mut self, revision: u64, route_rule_ids: Vec<Option<String>>) {
        self.revision = revision;
        self.mappings.insert(revision, route_rule_ids);
    }

    pub fn ingest(&mut self, raw_line: &str) {
        let line = strip_ansi(raw_line);
        let Some(connection_id) = connection_id(&line) else {
            return;
        };
        let revision = self.revision;
        let pending = self
            .pending
            .entry(connection_id)
            .or_insert_with(|| PendingConnection {
                revision,
                ..PendingConnection::default()
            });

        if let Some(target) = target(&line) {
            pending.target = Some(target);
        }
        if let Some(outbound) = outbound(&line) {
            pending.outbound = outbound;
        }
        if let Some(index) = matched_rule_index(&line) {
            pending.rule = self
                .mappings
                .get(&pending.revision)
                .and_then(|mapping| mapping.get(index))
                .and_then(Clone::clone);
        }

        let result = if line.contains("connection download finished") {
            Some(ConnectionResult::Success)
        } else if line.contains("ERROR") {
            Some(ConnectionResult::Failure(error_detail(&line)))
        } else {
            None
        };
        if let Some(result) = result {
            self.finish(connection_id, result);
        }
    }

    pub fn events(&self) -> Vec<ConnectionEvent> {
        self.events.iter().cloned().collect()
    }

    pub fn clear(&mut self) -> io::Result<()> {
        self.events.clear();
        self.pending.clear();
        if let Some(store) = &self.store {
            store.clear()?;
        }
        self.storage_error = None;
        Ok(())
    }

    pub fn storage_error(&self) -> Option<&str> {
        self.storage_error.as_deref()
    }

    fn finish(&mut self, connection_id: u64, result: ConnectionResult) {
        let Some(pending) = self.pending.remove(&connection_id) else {
            return;
        };
        let Some((target, port)) = pending.target else {
            return;
        };
        let observed_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let result = match result {
            ConnectionResult::Failure(detail) => {
                ConnectionResult::Failure(redact_sensitive(&detail))
            }
            ConnectionResult::Success => ConnectionResult::Success,
        };
        self.events.push_back(ConnectionEvent {
            observed_at_unix_ms,
            connection_id,
            target,
            port,
            outbound: pending.outbound,
            rule: pending
                .rule
                .map(RuleAttribution::Known)
                .unwrap_or(RuleAttribution::Unknown),
            result,
            config_revision: pending.revision,
        });
        if let Some(store) = &self.store
            && let Err(error) = store.persist(&mut self.events)
        {
            self.storage_error = Some(error.to_string());
        }
    }
}

pub fn redact_sensitive(value: &str) -> String {
    let mut redacted = redact_url_userinfo(value);
    for key in [
        "password",
        "passwd",
        "username",
        "authorization",
        "proxy-authorization",
    ] {
        redacted = redact_key(&redacted, key);
    }
    redacted
}

fn redact_url_userinfo(value: &str) -> String {
    let mut result = value.to_owned();
    let mut search_from = 0;
    while let Some(scheme) = result[search_from..].find("://") {
        let start = search_from + scheme + 3;
        let authority_end = result[start..]
            .find(['/', ' ', ')'])
            .map_or(result.len(), |offset| start + offset);
        let Some(at) = result[start..authority_end].rfind('@') else {
            search_from = authority_end;
            continue;
        };
        let end = start + at;
        result.replace_range(start..end, "[REDACTED]");
        search_from = start + "[REDACTED]@".len();
    }
    result
}

fn redact_key(value: &str, key: &str) -> String {
    let mut result = value.to_owned();
    let mut search_from = 0;
    loop {
        let lower = result.to_ascii_lowercase();
        let Some(offset) = lower[search_from..].find(key) else {
            break;
        };
        let key_start = search_from + offset;
        let mut cursor = key_start + key.len();
        while result
            .as_bytes()
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_whitespace() || matches!(*byte, b'"' | b'\''))
        {
            cursor += 1;
        }
        if !matches!(result.as_bytes().get(cursor), Some(b':' | b'=')) {
            search_from = cursor;
            continue;
        }
        cursor += 1;
        while result
            .as_bytes()
            .get(cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            cursor += 1;
        }
        let quote = result
            .as_bytes()
            .get(cursor)
            .copied()
            .filter(|byte| matches!(*byte, b'"' | b'\''));
        if quote.is_some() {
            cursor += 1;
        }
        let end = if let Some(quote) = quote {
            result[cursor..]
                .find(quote as char)
                .map_or(result.len(), |offset| cursor + offset)
        } else if key.contains("authorization") {
            result[cursor..]
                .find([',', '|', ')'])
                .map_or(result.len(), |offset| cursor + offset)
        } else {
            result[cursor..]
                .find(|character: char| {
                    character.is_whitespace() || matches!(character, ',' | ')' | '}')
                })
                .map_or(result.len(), |offset| cursor + offset)
        };
        result.replace_range(cursor..end, "[REDACTED]");
        search_from = cursor + "[REDACTED]".len();
    }
    result
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn connection_id(line: &str) -> Option<u64> {
    let mut rest = line;
    while let Some(start) = rest.find('[') {
        rest = &rest[start + 1..];
        let end = rest.find(']')?;
        let content = &rest[..end];
        if let Some((candidate, _)) = content.split_once(' ')
            && candidate.bytes().all(|byte| byte.is_ascii_digit())
        {
            return candidate.parse().ok();
        }
        rest = &rest[end + 1..];
    }
    None
}

fn target(line: &str) -> Option<(String, u16)> {
    let value = ["inbound connection to ", "inbound packet connection to "]
        .into_iter()
        .find_map(|marker| line.split_once(marker).map(|(_, value)| value.trim()))?;
    if let Ok(address) = value.parse::<std::net::SocketAddr>() {
        return Some((address.ip().to_string(), address.port()));
    }
    let (host, port) = value.rsplit_once(':')?;
    let port = port.parse().ok()?;
    Some((host.trim_matches(['[', ']']).to_owned(), port))
}

fn outbound(line: &str) -> Option<ConnectionOutbound> {
    let (_, value) = line.split_once("outbound/")?;
    let tag_start = value.find('[')? + 1;
    let tag_end = value[tag_start..].find(']')? + tag_start;
    Some(match &value[tag_start..tag_end] {
        "direct" => ConnectionOutbound::Direct,
        "proxy" => ConnectionOutbound::Proxy,
        _ => ConnectionOutbound::Unknown,
    })
}

fn matched_rule_index(line: &str) -> Option<usize> {
    let (_, value) = line.split_once("router: match[")?;
    value.split_once(']')?.0.parse().ok()
}

fn error_detail(line: &str) -> String {
    line.split_once("connection: ")
        .map(|(_, detail)| detail.trim().to_owned())
        .unwrap_or_else(|| "连接失败".into())
}

fn strip_ansi(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn event(timestamp: u128, detail: &str) -> ConnectionEvent {
        ConnectionEvent {
            observed_at_unix_ms: timestamp,
            connection_id: timestamp as u64,
            target: "example.com".into(),
            port: 443,
            outbound: ConnectionOutbound::Proxy,
            rule: RuleAttribution::Unknown,
            result: ConnectionResult::Failure(detail.into()),
            config_revision: 1,
        }
    }

    #[test]
    fn real_core_sample_maps_verified_rule_and_success() {
        let mut adapter = ConnectionLogAdapter::default();
        adapter.begin_revision(7, vec![None, None, Some("stable-rule-id".into())]);
        for line in [
            "\x1b[36mINFO\x1b[0m[0001] [\x1b[38;5;34m3109887250\x1b[0m 0ms] inbound/tun[tun-in]: inbound connection to 198.19.0.1:443",
            "\x1b[37mDEBUG\x1b[0m[0001] [\x1b[38;5;34m3109887250\x1b[0m 0ms] router: match[2] domain=example.com => route(proxy)",
            "\x1b[36mINFO\x1b[0m[0001] [\x1b[38;5;34m3109887250\x1b[0m 0ms] outbound/socks[proxy]: outbound connection to example.com:443",
            "\x1b[37mDEBUG\x1b[0m[0001] [\x1b[38;5;34m3109887250\x1b[0m 6ms] connection: connection download finished",
        ] {
            adapter.ingest(line);
        }
        let events = adapter.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].target, "198.19.0.1");
        assert_eq!(events[0].port, 443);
        assert_eq!(events[0].outbound, ConnectionOutbound::Proxy);
        assert_eq!(
            events[0].rule,
            RuleAttribution::Known("stable-rule-id".into())
        );
        assert_eq!(events[0].result, ConnectionResult::Success);
        assert_eq!(events[0].config_revision, 7);
    }

    #[test]
    fn actual_outbound_without_match_stays_unknown_and_failure_is_diagnostic() {
        let mut adapter = ConnectionLogAdapter::default();
        adapter.begin_revision(9, vec![Some("must-not-be-guessed".into())]);
        for line in [
            "INFO[0001] [424138285 0ms] inbound/mixed[0]: inbound connection to 127.0.0.1:18080",
            "INFO[0001] [424138285 0ms] outbound/http[proxy]: outbound connection to 127.0.0.1:18080",
            "ERROR[0001] [424138285 1ms] connection: open connection using outbound/http[proxy]: authentication required",
        ] {
            adapter.ingest(line);
        }
        let events = adapter.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].rule, RuleAttribution::Unknown);
        assert_eq!(events[0].outbound, ConnectionOutbound::Proxy);
        assert!(matches!(
            &events[0].result,
            ConnectionResult::Failure(detail) if detail.contains("authentication required")
        ));
    }

    #[test]
    fn mappings_are_bound_to_the_revision_that_started_the_connection() {
        let mut adapter = ConnectionLogAdapter::default();
        adapter.begin_revision(1, vec![Some("old-rule".into())]);
        adapter.ingest(
            "INFO[0001] [10 0ms] inbound/tun[tun-in]: inbound connection to example.com:80",
        );
        adapter.begin_revision(2, vec![Some("new-rule".into())]);
        adapter.ingest("DEBUG[0001] [10 1ms] router: match[0] domain=example.com => route(proxy)");
        adapter.ingest(
            "INFO[0001] [10 1ms] outbound/socks[proxy]: outbound connection to example.com:80",
        );
        adapter.ingest("DEBUG[0001] [10 2ms] connection: connection download finished");
        assert_eq!(
            adapter.events()[0].rule,
            RuleAttribution::Known("old-rule".into())
        );
    }

    #[test]
    fn sensitive_fields_and_url_userinfo_are_redacted() {
        let input = "password=secret username='alice' Authorization: Basic dXNlcjpwYXNz, proxy-authorization=Bearer token https://bob:hunter2@example.com/path";
        let output = redact_sensitive(input);
        for secret in ["secret", "alice", "dXNlcjpwYXNz", "token", "bob", "hunter2"] {
            assert!(!output.contains(secret), "secret leaked: {secret}");
        }
        assert!(output.contains("[REDACTED]"));
    }

    #[test]
    fn persisted_events_are_redacted_reloadable_and_clearable() {
        let root = std::env::temp_dir().join(format!("socks-proxy-logs-{}", Uuid::new_v4()));
        let path = root.join("connections.jsonl");
        let mut adapter = ConnectionLogAdapter::with_store(&path);
        adapter.begin_revision(1, vec![]);
        adapter.ingest(
            "INFO[0001] [42 0ms] inbound/tun[tun-in]: inbound connection to example.com:443",
        );
        adapter.ingest(
            "INFO[0001] [42 0ms] outbound/http[proxy]: outbound connection to example.com:443",
        );
        adapter.ingest("ERROR[0001] [42 1ms] connection: password=fixture-secret");
        assert_eq!(adapter.events().len(), 1);
        let disk = fs::read_to_string(&path).unwrap();
        assert!(!disk.contains("fixture-secret"));
        assert!(disk.contains("[REDACTED]"));

        let mut reloaded = ConnectionLogAdapter::with_store(&path);
        assert_eq!(reloaded.events().len(), 1);
        reloaded.clear().unwrap();
        assert!(reloaded.events().is_empty());
        assert!(!path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn store_enforces_age_and_size_limits_by_removing_oldest_events() {
        assert_eq!(DEFAULT_MAX_BYTES, 20 * 1024 * 1024);
        assert_eq!(DEFAULT_RETENTION, Duration::from_secs(7 * 24 * 60 * 60));
        let root = std::env::temp_dir().join(format!("socks-proxy-rotation-{}", Uuid::new_v4()));
        let path = root.join("connections.jsonl");
        let store = ConnectionLogStore {
            path: path.clone(),
            max_bytes: 450,
            retention: Duration::from_secs(60),
        };
        let now = now_ms();
        let mut events = VecDeque::from([
            event(now.saturating_sub(120_000), "expired"),
            event(now.saturating_sub(2_000), &"a".repeat(180)),
            event(now.saturating_sub(1_000), &"b".repeat(180)),
            event(now, "newest"),
        ]);
        store.persist(&mut events).unwrap();
        assert!(events.iter().all(|item| {
            item.observed_at_unix_ms >= now.saturating_sub(60_000)
                && !matches!(&item.result, ConnectionResult::Failure(detail) if detail == "expired")
        }));
        assert_eq!(
            events.back().unwrap().result,
            ConnectionResult::Failure("newest".into())
        );
        assert!(fs::metadata(&path).unwrap().len() <= 450);
        let _ = fs::remove_dir_all(root);
    }
}

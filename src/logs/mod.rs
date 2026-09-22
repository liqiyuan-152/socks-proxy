use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::Duration,
    time::{SystemTime, UNIX_EPOCH},
};

const DEFAULT_MAX_BYTES: usize = 20 * 1024 * 1024;
const DEFAULT_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const MEMORY_WINDOW: usize = 500;
pub const LOG_PAGE_SIZE: usize = 100;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionLogSegment {
    Current,
    Previous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionLogCursor {
    segment: ConnectionLogSegment,
    end_offset: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectionLogFilter {
    pub query: String,
    pub outbound: Option<ConnectionOutbound>,
    pub success: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionLogPage {
    pub events: Vec<ConnectionEvent>,
    pub next_cursor: Option<ConnectionLogCursor>,
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

    fn load_recent(&self) -> io::Result<VecDeque<ConnectionEvent>> {
        let previous = self.path.with_extension("jsonl.previous");
        let mut events = self.load_recent_file(&previous, MEMORY_WINDOW)?;
        let current = self.load_recent_file(&self.path, MEMORY_WINDOW)?;
        events.extend(current);
        while events.len() > MEMORY_WINDOW {
            events.pop_front();
        }
        Ok(events)
    }

    fn load_recent_file(&self, path: &Path, limit: usize) -> io::Result<VecDeque<ConnectionEvent>> {
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(VecDeque::new()),
            Err(error) => return Err(error),
        };
        let mut position = file.metadata()?.len();
        let mut tail = Vec::new();
        let mut events = VecDeque::new();
        while position > 0 && events.len() < limit {
            let chunk = position.min(8192) as usize;
            position -= chunk as u64;
            file.seek(SeekFrom::Start(position))?;
            let mut bytes = vec![0; chunk];
            file.read_exact(&mut bytes)?;
            bytes.extend_from_slice(&tail);
            let mut lines = bytes.split(|byte| *byte == b'\n').collect::<Vec<_>>();
            tail = lines.remove(0).to_vec();
            for line in lines.into_iter().rev() {
                if let Ok(event) = serde_json::from_slice(line) {
                    events.push_front(redact_event(event));
                    if events.len() == limit {
                        break;
                    }
                }
            }
        }
        if events.len() < limit
            && !tail.is_empty()
            && let Ok(event) = serde_json::from_slice(&tail)
        {
            events.push_front(redact_event(event));
        }
        Ok(events)
    }

    fn page(
        &self,
        cursor: Option<ConnectionLogCursor>,
        filter: &ConnectionLogFilter,
        limit: usize,
    ) -> io::Result<ConnectionLogPage> {
        let (segment, path, end_offset) = match cursor {
            Some(ConnectionLogCursor {
                segment,
                end_offset,
            }) => (segment, self.path_for(segment), end_offset),
            None => {
                let path = self.path_for(ConnectionLogSegment::Current);
                let end_offset = fs::metadata(&path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                (ConnectionLogSegment::Current, path, end_offset)
            }
        };
        let (events, next_offset) = self.reverse_page(&path, end_offset, filter, limit)?;
        if let Some(end_offset) = next_offset {
            return Ok(ConnectionLogPage {
                events,
                next_cursor: Some(ConnectionLogCursor {
                    segment,
                    end_offset,
                }),
            });
        }
        if segment == ConnectionLogSegment::Current {
            let previous = self.path_for(ConnectionLogSegment::Previous);
            if let Ok(metadata) = fs::metadata(&previous)
                && metadata.len() > 0
            {
                return Ok(ConnectionLogPage {
                    events,
                    next_cursor: Some(ConnectionLogCursor {
                        segment: ConnectionLogSegment::Previous,
                        end_offset: metadata.len(),
                    }),
                });
            }
        }
        Ok(ConnectionLogPage {
            events,
            next_cursor: None,
        })
    }

    fn reverse_page(
        &self,
        path: &Path,
        end_offset: u64,
        filter: &ConnectionLogFilter,
        limit: usize,
    ) -> io::Result<(Vec<ConnectionEvent>, Option<u64>)> {
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), None)),
            Err(error) => return Err(error),
        };
        let mut position = end_offset.min(file.metadata()?.len());
        let mut tail = Vec::new();
        let mut events = Vec::with_capacity(limit);
        let mut resume = None;
        while position > 0 && events.len() < limit {
            let length = position.min(8192) as usize;
            position -= length as u64;
            file.seek(SeekFrom::Start(position))?;
            let mut bytes = vec![0; length];
            file.read_exact(&mut bytes)?;
            bytes.extend_from_slice(&tail);
            let mut line_end = bytes.len();
            for index in (0..bytes.len()).rev() {
                if bytes[index] == b'\n' {
                    if index + 1 < line_end {
                        let line_start = position + index as u64 + 1;
                        resume = Some(line_start);
                        if let Ok(event) = serde_json::from_slice(&bytes[index + 1..line_end]) {
                            let event = redact_event(event);
                            if filter.matches(&event) {
                                events.push(event);
                                if events.len() == limit {
                                    return Ok((events, resume));
                                }
                            }
                        }
                    }
                    line_end = index;
                }
            }
            tail = bytes[..line_end].to_vec();
            if position == 0 {
                if !tail.is_empty()
                    && let Ok(event) = serde_json::from_slice(&tail)
                {
                    let event = redact_event(event);
                    if filter.matches(&event) {
                        events.push(event);
                    }
                }
                return Ok((events, None));
            }
        }
        Ok((events, resume))
    }

    fn path_for(&self, segment: ConnectionLogSegment) -> PathBuf {
        match segment {
            ConnectionLogSegment::Current => self.path.clone(),
            ConnectionLogSegment::Previous => self.path.with_extension("jsonl.previous"),
        }
    }

    fn append(&self, event: &ConnectionEvent) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        self.remove_expired_segment()?;
        let mut record = Vec::new();
        serde_json::to_writer(&mut record, event).map_err(io::Error::other)?;
        record.push(b'\n');
        if self.path.exists()
            && fs::metadata(&self.path)?
                .len()
                .saturating_add(record.len() as u64)
                > self.max_bytes as u64
        {
            let previous = self.path.with_extension("jsonl.previous");
            let _ = fs::remove_file(&previous);
            fs::rename(&self.path, previous)?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(&record)?;
        file.sync_data()
    }

    fn remove_expired_segment(&self) -> io::Result<()> {
        let previous = self.path.with_extension("jsonl.previous");
        if previous.exists()
            && previous
                .metadata()?
                .modified()
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > self.retention)
        {
            fs::remove_file(previous)?;
        }
        Ok(())
    }

    fn clear(&self) -> io::Result<()> {
        for path in [&self.path, &self.path.with_extension("jsonl.previous")] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

impl ConnectionLogFilter {
    fn matches(&self, event: &ConnectionEvent) -> bool {
        let query = self.query.trim().to_ascii_lowercase();
        let query_matches = query.is_empty()
            || event.target.to_ascii_lowercase().contains(&query)
            || matches!(&event.rule, RuleAttribution::Known(rule) if rule.to_ascii_lowercase().contains(&query));
        let outbound_matches = self
            .outbound
            .is_none_or(|outbound| outbound == event.outbound);
        let success_matches = self
            .success
            .is_none_or(|success| success == matches!(event.result, ConnectionResult::Success));
        query_matches && outbound_matches && success_matches
    }
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
        let (events, storage_error) = match store.load_recent() {
            Ok(events) => (events, None),
            Err(error) => (VecDeque::new(), Some(error.to_string())),
        };
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

    pub fn page(
        &self,
        cursor: Option<ConnectionLogCursor>,
        filter: &ConnectionLogFilter,
        limit: usize,
    ) -> io::Result<ConnectionLogPage> {
        match &self.store {
            Some(store) => store.page(cursor, filter, limit),
            None => Ok(ConnectionLogPage {
                events: self
                    .events
                    .iter()
                    .rev()
                    .filter(|event| filter.matches(event))
                    .take(limit)
                    .cloned()
                    .collect(),
                next_cursor: None,
            }),
        }
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
        while self.events.len() > MEMORY_WINDOW {
            self.events.pop_front();
        }
        if let Some(store) = &self.store
            && let Err(error) = store.append(self.events.back().expect("event was appended"))
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

#[cfg(test)]
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
        store
            .append(&event(now.saturating_sub(120_000), "expired"))
            .unwrap();
        store
            .append(&event(now.saturating_sub(2_000), &"a".repeat(180)))
            .unwrap();
        store
            .append(&event(now.saturating_sub(1_000), &"b".repeat(180)))
            .unwrap();
        store.append(&event(now, "newest")).unwrap();
        assert!(fs::metadata(&path).unwrap().len() <= 450);
        let recent = store.load_recent().unwrap();
        assert_eq!(
            recent.back().unwrap().result,
            ConnectionResult::Failure("newest".into())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reloads_recent_events_from_current_and_previous_segment() {
        let root = std::env::temp_dir().join(format!("socks-proxy-segments-{}", Uuid::new_v4()));
        let path = root.join("connections.jsonl");
        fs::create_dir_all(&root).unwrap();
        let older = event(1, "older");
        let newer = event(2, "newer");
        let mut old_bytes = serde_json::to_vec(&older).unwrap();
        old_bytes.push(b'\n');
        let mut new_bytes = serde_json::to_vec(&newer).unwrap();
        new_bytes.push(b'\n');
        fs::write(path.with_extension("jsonl.previous"), old_bytes).unwrap();
        fs::write(&path, new_bytes).unwrap();

        let loaded = ConnectionLogStore::new(&path).load_recent().unwrap();
        assert_eq!(
            loaded
                .iter()
                .map(|item| item.connection_id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pages_events_in_reverse_order_without_replaying_records() {
        let root = std::env::temp_dir().join(format!("socks-proxy-pages-{}", Uuid::new_v4()));
        let path = root.join("connections.jsonl");
        fs::create_dir_all(&root).unwrap();
        let mut bytes = Vec::new();
        for timestamp in 1..=5 {
            serde_json::to_writer(&mut bytes, &event(timestamp, &format!("event-{timestamp}")))
                .unwrap();
            bytes.push(b'\n');
        }
        fs::write(&path, bytes).unwrap();
        let store = ConnectionLogStore::new(&path);
        let filter = ConnectionLogFilter::default();
        let first = store.page(None, &filter, 2).unwrap();
        assert_eq!(
            first
                .events
                .iter()
                .map(|event| event.connection_id)
                .collect::<Vec<_>>(),
            vec![5, 4]
        );
        let second = store.page(first.next_cursor, &filter, 2).unwrap();
        assert_eq!(
            second
                .events
                .iter()
                .map(|event| event.connection_id)
                .collect::<Vec<_>>(),
            vec![3, 2]
        );
        let third = store.page(second.next_cursor, &filter, 2).unwrap();
        assert_eq!(
            third
                .events
                .iter()
                .map(|event| event.connection_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert!(third.next_cursor.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn filtered_page_advances_past_non_matching_records() {
        let root =
            std::env::temp_dir().join(format!("socks-proxy-filter-pages-{}", Uuid::new_v4()));
        let path = root.join("connections.jsonl");
        fs::create_dir_all(&root).unwrap();
        let mut bytes = Vec::new();
        for timestamp in 1..=6 {
            let mut item = event(timestamp, &format!("event-{timestamp}"));
            item.outbound = if timestamp % 2 == 0 {
                ConnectionOutbound::Proxy
            } else {
                ConnectionOutbound::Direct
            };
            serde_json::to_writer(&mut bytes, &item).unwrap();
            bytes.push(b'\n');
        }
        fs::write(&path, bytes).unwrap();
        let store = ConnectionLogStore::new(&path);
        let filter = ConnectionLogFilter {
            outbound: Some(ConnectionOutbound::Proxy),
            ..ConnectionLogFilter::default()
        };
        let first = store.page(None, &filter, 2).unwrap();
        assert_eq!(
            first
                .events
                .iter()
                .map(|event| event.connection_id)
                .collect::<Vec<_>>(),
            vec![6, 4]
        );
        let second = store.page(first.next_cursor, &filter, 2).unwrap();
        assert_eq!(
            second
                .events
                .iter()
                .map(|event| event.connection_id)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert!(second.next_cursor.is_none());
        let _ = fs::remove_dir_all(root);
    }
}

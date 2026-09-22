use crate::logs::{self, ConnectionEvent, ConnectionOutbound, ConnectionResult, RuleAttribution};

pub fn outbound_label(outbound: ConnectionOutbound) -> &'static str {
    match outbound {
        ConnectionOutbound::Direct => "直连",
        ConnectionOutbound::Proxy => "代理",
        ConnectionOutbound::Unknown => "未知",
    }
}

pub fn detail_values(
    event: &ConnectionEvent,
    event_time: impl Fn(u128) -> String,
) -> [(&'static str, String); 6] {
    let rule = match &event.rule {
        RuleAttribution::Known(rule) => rule.clone(),
        RuleAttribution::Unknown => "未知".into(),
    };
    let result = match &event.result {
        ConnectionResult::Success => "成功".into(),
        ConnectionResult::Failure(detail) => logs::redact_sensitive(detail),
    };
    [
        ("时间", event_time(event.observed_at_unix_ms)),
        ("目标", event.target.clone()),
        ("端口", event.port.to_string()),
        ("出站", outbound_label(event.outbound).into()),
        ("规则", rule),
        ("结果", result),
    ]
}

use crate::domain::{PortSet, RoutingRule, RuleTarget};

pub fn matches(rule: &RoutingRule, filter: &str) -> bool {
    let needle = filter.trim().to_lowercase();
    needle.is_empty()
        || rule.name.to_lowercase().contains(&needle)
        || target_label(&rule.target).to_lowercase().contains(&needle)
        || rule.note.to_lowercase().contains(&needle)
}

pub fn target_label(target: &RuleTarget) -> String {
    match target {
        RuleTarget::Domain(value) => value.as_str().to_owned(),
        RuleTarget::DomainSuffix(value) => format!("*.{}", value.as_str()),
        RuleTarget::Ip(value) => value.to_string(),
        RuleTarget::Cidr(value) => value.to_string(),
        RuleTarget::Range(value) => value.to_string(),
    }
}

pub fn ports_label(ports: &PortSet) -> String {
    if ports.intervals() == [(1, u16::MAX)] {
        return "全部".into();
    }
    ports
        .intervals()
        .iter()
        .map(|(start, end)| {
            if start == end {
                start.to_string()
            } else {
                format!("{start}-{end}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

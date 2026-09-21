use crate::domain::{DomainName, ProxyHost, RoutingRule, RuleTarget};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingMode {
    Direct,
    Rules,
    GlobalProxy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteDecision {
    Direct,
    Proxy,
    NeedsDirectDns,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionTarget {
    pub domain: Option<DomainName>,
    pub real_candidates: Vec<IpAddr>,
    pub port: u16,
}

#[derive(Debug, Clone, Default)]
pub struct BaseExceptions {
    pub control_endpoints: Vec<SocketAddr>,
    pub upstream_host: Option<ProxyHost>,
    pub upstream_port: Option<u16>,
    pub upstream_addresses: Vec<IpAddr>,
}

impl BaseExceptions {
    fn matches(&self, target: &ConnectionTarget) -> bool {
        if target.real_candidates.iter().any(|ip| {
            self.control_endpoints
                .contains(&SocketAddr::new(*ip, target.port))
        }) {
            return true;
        }
        if self.upstream_port != Some(target.port) {
            return false;
        }
        match &self.upstream_host {
            Some(ProxyHost::Ip(ip)) => target.real_candidates.contains(ip),
            Some(ProxyHost::Domain(domain)) => {
                target.domain.as_ref() == Some(domain)
                    || target
                        .real_candidates
                        .iter()
                        .any(|ip| self.upstream_addresses.contains(ip))
            }
            None => false,
        }
    }
}

pub fn decide(
    mode: RoutingMode,
    target: &ConnectionTarget,
    rules: &[RoutingRule],
    exceptions: &BaseExceptions,
) -> RouteDecision {
    if mode == RoutingMode::Direct || exceptions.matches(target) {
        return RouteDecision::Direct;
    }

    match mode {
        RoutingMode::Direct => RouteDecision::Direct,
        RoutingMode::GlobalProxy => decide_global(target),
        RoutingMode::Rules => decide_rules(target, rules),
    }
}

fn decide_global(target: &ConnectionTarget) -> RouteDecision {
    if target.real_candidates.is_empty() {
        return if target.domain.is_some() {
            RouteDecision::NeedsDirectDns
        } else {
            RouteDecision::Proxy
        };
    }
    if target
        .real_candidates
        .iter()
        .all(|ip| is_local_or_private(*ip))
    {
        RouteDecision::Direct
    } else {
        RouteDecision::Proxy
    }
}

fn decide_rules(target: &ConnectionTarget, rules: &[RoutingRule]) -> RouteDecision {
    if target.domain.as_ref().is_some_and(|domain| {
        rules
            .iter()
            .any(|rule| rule.matches_domain(domain, target.port))
    }) {
        return RouteDecision::Proxy;
    }

    let has_applicable_ip_rule = rules.iter().any(|rule| {
        rule.enabled
            && rule.ports.contains(target.port)
            && matches!(
                rule.target,
                RuleTarget::Ip(_) | RuleTarget::Cidr(_) | RuleTarget::Range(_)
            )
    });
    if target.domain.is_some() && has_applicable_ip_rule && target.real_candidates.is_empty() {
        return RouteDecision::NeedsDirectDns;
    }
    if target.real_candidates.iter().any(|candidate| {
        rules
            .iter()
            .any(|rule| rule.matches_ip(*candidate, target.port))
    }) {
        RouteDecision::Proxy
    } else {
        RouteDecision::Direct
    }
}

fn is_local_or_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_private_v4(ip),
        IpAddr::V6(ip) => is_private_v6(ip),
    }
}

fn is_private_v4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_private()
        || octets[0] == 100 && (64..=127).contains(&octets[1])
        || octets[0] == 198 && (18..=19).contains(&octets[1])
}

fn is_private_v6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    ip.is_unspecified()
        || ip.is_loopback()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
}

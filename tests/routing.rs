use socks_proxy::{
    domain::{DomainName, ProxyHost, RoutingRule, RuleTarget},
    routing::{BaseExceptions, ConnectionTarget, RouteDecision, RoutingMode, decide},
};
use std::net::IpAddr;

fn target(domain: Option<&str>, ips: &[&str], port: u16) -> ConnectionTarget {
    ConnectionTarget {
        domain: domain.map(|value| DomainName::parse(value).unwrap()),
        real_candidates: ips.iter().map(|value| value.parse().unwrap()).collect(),
        port,
    }
}

fn rule(target: RuleTarget, ports: &str) -> RoutingRule {
    RoutingRule {
        id: "rule".into(),
        name: "rule".into(),
        enabled: true,
        target,
        ports: ports.parse().unwrap(),
        note: String::new(),
    }
}

#[test]
fn rule_fields_are_and_rules_are_or() {
    let rules = [
        rule(RuleTarget::Ip("203.0.113.10".parse().unwrap()), "22"),
        rule(
            RuleTarget::Domain(DomainName::parse("example.com").unwrap()),
            "443",
        ),
    ];
    let exceptions = BaseExceptions::default();
    assert_eq!(
        decide(
            RoutingMode::Rules,
            &target(None, &["203.0.113.10"], 22),
            &rules,
            &exceptions
        ),
        RouteDecision::Proxy
    );
    assert_eq!(
        decide(
            RoutingMode::Rules,
            &target(None, &["203.0.113.10"], 443),
            &rules,
            &exceptions
        ),
        RouteDecision::Direct
    );
    assert_eq!(
        decide(
            RoutingMode::Rules,
            &target(Some("example.com"), &[], 443),
            &rules,
            &exceptions
        ),
        RouteDecision::Proxy
    );
}

#[test]
fn rules_can_proxy_remote_private_addresses() {
    let rules = [rule(RuleTarget::Ip("10.20.30.40".parse().unwrap()), "22")];
    assert_eq!(
        decide(
            RoutingMode::Rules,
            &target(None, &["10.20.30.40"], 22),
            &rules,
            &BaseExceptions::default()
        ),
        RouteDecision::Proxy
    );
}

#[test]
fn global_mode_directs_only_all_private_candidate_sets() {
    let exceptions = BaseExceptions::default();
    assert_eq!(
        decide(
            RoutingMode::GlobalProxy,
            &target(Some("lan.example"), &["10.0.0.2", "fd00::2"], 443),
            &[],
            &exceptions
        ),
        RouteDecision::Direct
    );
    assert_eq!(
        decide(
            RoutingMode::GlobalProxy,
            &target(
                Some("mixed.example"),
                &["10.0.0.2", "2001:4860:4860::8888"],
                443
            ),
            &[],
            &exceptions
        ),
        RouteDecision::Proxy
    );
    assert_eq!(
        decide(
            RoutingMode::GlobalProxy,
            &target(Some("unresolved.example"), &[], 443),
            &[],
            &exceptions
        ),
        RouteDecision::NeedsDirectDns
    );
}

#[test]
fn base_exceptions_precede_user_rules_and_global_mode() {
    let upstream_ip: IpAddr = "203.0.113.9".parse().unwrap();
    let control_ip: IpAddr = "127.0.0.1".parse().unwrap();
    let exceptions = BaseExceptions {
        control_endpoints: vec![(control_ip, 9090).into()],
        upstream_host: Some(ProxyHost::parse("proxy.example.com").unwrap()),
        upstream_port: Some(1080),
        upstream_addresses: vec![upstream_ip],
    };
    let catch_all = [rule(RuleTarget::Cidr("0.0.0.0/0".parse().unwrap()), "")];
    assert_eq!(
        decide(
            RoutingMode::Rules,
            &target(None, &["127.0.0.1"], 9090),
            &catch_all,
            &exceptions
        ),
        RouteDecision::Direct
    );
    assert_eq!(
        decide(
            RoutingMode::Rules,
            &target(Some("proxy.example.com"), &["203.0.113.9"], 1080),
            &catch_all,
            &exceptions
        ),
        RouteDecision::Direct
    );
    assert_eq!(
        decide(
            RoutingMode::GlobalProxy,
            &target(None, &["203.0.113.9"], 1080),
            &[],
            &exceptions
        ),
        RouteDecision::Direct
    );
}

#[test]
fn ip_rules_request_resolution_and_any_candidate_can_match() {
    let rules = [rule(
        RuleTarget::Cidr("203.0.113.0/24".parse().unwrap()),
        "443",
    )];
    assert_eq!(
        decide(
            RoutingMode::Rules,
            &target(Some("site.example"), &[], 443),
            &rules,
            &BaseExceptions::default()
        ),
        RouteDecision::NeedsDirectDns
    );
    assert_eq!(
        decide(
            RoutingMode::Rules,
            &target(Some("site.example"), &["192.0.2.1", "203.0.113.5"], 443),
            &rules,
            &BaseExceptions::default()
        ),
        RouteDecision::Proxy
    );
}

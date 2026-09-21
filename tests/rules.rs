use socks_proxy::domain::{DomainName, IpRange, RoutingRule, RuleTarget};
fn domain(s: &str) -> DomainName {
    DomainName::parse(s).unwrap()
}
#[test]
fn domain_normalization_and_label_boundaries() {
    assert_eq!(domain(" EXAMPLE.com. "), domain("example.com"));
    assert_eq!(domain("例子.测试"), domain("xn--fsqu00a.xn--0zwm56d"));
    for bad in [
        "",
        "a..com",
        "https://example.com",
        "example.com:443",
        "a_b.com",
        "-a.com",
        "a-.com",
        "127.0.0.1",
        "::1",
        "example.com..",
    ] {
        assert!(DomainName::parse(bad).is_err(), "{bad}");
    }
    let exact = RuleTarget::Domain(domain("example.com"));
    let suffix = RuleTarget::DomainSuffix(domain("example.com"));
    assert!(exact.matches_domain(&domain("example.com")));
    assert!(!exact.matches_domain(&domain("a.example.com")));
    assert!(suffix.matches_domain(&domain("a.example.com")));
    assert!(suffix.matches_domain(&domain("example.com")));
    assert!(!suffix.matches_domain(&domain("badexample.com")));
}
#[test]
fn five_targets_ports_and_disabled_rule() {
    let targets = [
        RuleTarget::Domain(domain("example.com")),
        RuleTarget::DomainSuffix(domain("example.com")),
        RuleTarget::Ip("2001:db8::10".parse().unwrap()),
        RuleTarget::Cidr("2001:db8::/64".parse().unwrap()),
        RuleTarget::Range(
            IpRange::new(
                "2001:db8::a".parse().unwrap(),
                "2001:db8::1e".parse().unwrap(),
            )
            .unwrap(),
        ),
    ];
    for target in targets {
        let mut rule = RoutingRule {
            id: "test".into(),
            name: "test".into(),
            enabled: true,
            target,
            ports: "22,443".parse().unwrap(),
            note: String::new(),
        };
        let matches = |r: &RoutingRule, p| {
            r.matches_domain(&domain("example.com"), p)
                || r.matches_ip("2001:0db8:0:0:0:0:0:10".parse().unwrap(), p)
        };
        assert!(matches(&rule, 22));
        assert!(!matches(&rule, 80));
        rule.enabled = false;
        assert!(!matches(&rule, 22));
    }
}

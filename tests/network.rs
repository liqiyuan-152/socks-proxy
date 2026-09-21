use socks_proxy::domain::{IpNetwork, IpRange, PortSet};
use std::net::{IpAddr, Ipv4Addr};
fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}
#[test]
fn port_boundaries_and_normalization() {
    let ports: PortSet = "22,443,8000-9000,8999-9001,65535,1".parse().unwrap();
    for p in [1, 22, 443, 8000, 9000, 9001, 65535] {
        assert!(ports.contains(p));
    }
    for p in [0, 23, 442, 7999, 9002, 65534] {
        assert!(!ports.contains(p));
    }
    assert_eq!(
        ports.intervals(),
        &[(1, 1), (22, 22), (443, 443), (8000, 9001), (65535, 65535)]
    );
    let all: PortSet = "  ".parse().unwrap();
    assert!(!all.contains(0));
    assert!((1..=65535).all(|p| all.contains(p)));
    for bad in [
        "0",
        "65536",
        "9000-8000",
        "22,",
        ",22",
        "1--2",
        "+22",
        "-1",
        "1.5",
        "1, ,2",
    ] {
        assert!(bad.parse::<PortSet>().is_err(), "{bad}");
    }
}
#[test]
fn range_errors_and_full_address_spaces() {
    assert!(IpRange::new(ip("::"), ip("1.2.3.4")).is_err());
    assert!(IpRange::new(ip("::2"), ip("::1")).is_err());
    for (a, b, w) in [
        ("0.0.0.0", "255.255.255.255", 32),
        ("::", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", 128),
    ] {
        let blocks = IpRange::new(ip(a), ip(b)).unwrap().to_cidrs();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].prefix(), 0);
        assert!(blocks[0].contains(ip(a)) && blocks[0].contains(ip(b)));
        let single = IpRange::new(ip(b), ip(b)).unwrap().to_cidrs();
        assert_eq!(single[0].prefix(), w);
    }
}
#[test]
fn exhaustive_small_ranges_are_equivalent_and_disjoint() {
    for start in 0..64u32 {
        for end in start..64u32 {
            let range =
                IpRange::new(Ipv4Addr::from(start).into(), Ipv4Addr::from(end).into()).unwrap();
            let blocks = range.to_cidrs();
            assert!(blocks.len() <= 64);
            for candidate in 0..=64u32 {
                let candidate = Ipv4Addr::from(candidate).into();
                assert_eq!(
                    blocks.iter().filter(|b| b.contains(candidate)).count(),
                    usize::from(range.contains(candidate))
                );
            }
        }
    }
}
#[test]
fn ipv6_range_and_cidr_validation() {
    let range = IpRange::new(ip("2001:db8::a"), ip("2001:db8::1e")).unwrap();
    let blocks = range.to_cidrs();
    assert_eq!(
        blocks.iter().map(ToString::to_string).collect::<Vec<_>>(),
        [
            "2001:db8::a/127",
            "2001:db8::c/126",
            "2001:db8::10/125",
            "2001:db8::18/126",
            "2001:db8::1c/127",
            "2001:db8::1e/128"
        ]
    );
    for n in 0..40u128 {
        let candidate = IpAddr::V6((0x20010db8000000000000000000000000u128 + n).into());
        assert_eq!(
            blocks.iter().any(|b| b.contains(candidate)),
            (10..=30).contains(&n)
        );
    }
    for bad in ["1.2.3.4/33", "::/129", "::/-1", "1.2.3.4", "::/+1"] {
        assert!(bad.parse::<IpNetwork>().is_err());
    }
    assert_eq!(
        "192.0.2.15/24".parse::<IpNetwork>().unwrap().to_string(),
        "192.0.2.0/24"
    );
    assert!("::/0".parse::<IpNetwork>().unwrap().contains(ip("::1")));
    assert!(
        !"::/0"
            .parse::<IpNetwork>()
            .unwrap()
            .contains(ip("127.0.0.1"))
    );
}

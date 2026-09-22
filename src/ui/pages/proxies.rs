use crate::domain::{ProxyHost, ProxyProtocol};

pub fn protocol_label(protocol: ProxyProtocol) -> &'static str {
    match protocol {
        ProxyProtocol::Socks5 => "SOCKS5",
        ProxyProtocol::Http => "HTTP",
    }
}

pub fn host_label(host: &ProxyHost) -> String {
    match host {
        ProxyHost::Ip(ip) => ip.to_string(),
        ProxyHost::Domain(domain) => domain.as_str().to_owned(),
    }
}

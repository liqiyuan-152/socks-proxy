use super::{IpNetwork, IpRange, PortSet, ValidationError};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainName(String);
impl DomainName {
    pub fn parse(input: &str) -> Result<Self, ValidationError> {
        let input = input.trim().strip_suffix('.').unwrap_or(input.trim());
        let normalized = idna::domain_to_ascii_strict(input)
            .map_err(|_| ValidationError("域名格式无效"))?
            .to_ascii_lowercase();
        if normalized.is_empty()
            || normalized.len() > 253
            || normalized.parse::<IpAddr>().is_ok()
            || normalized.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            })
        {
            return Err(ValidationError("请输入有效域名，不含协议、路径或端口"));
        }
        Ok(Self(normalized))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn validate(&self) -> Result<(), ValidationError> {
        if Self::parse(&self.0)? != *self {
            return Err(ValidationError("域名未规范化"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleTarget {
    Domain(DomainName),
    DomainSuffix(DomainName),
    Ip(IpAddr),
    Cidr(IpNetwork),
    Range(IpRange),
}
impl RuleTarget {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Domain(domain) | Self::DomainSuffix(domain) => domain.validate(),
            Self::Ip(_) => Ok(()),
            Self::Cidr(network) => network.validate(),
            Self::Range(range) => range.validate(),
        }
    }

    pub fn matches_domain(&self, domain: &DomainName) -> bool {
        match self {
            Self::Domain(expected) => expected == domain,
            Self::DomainSuffix(expected) => {
                expected == domain
                    || domain
                        .0
                        .strip_suffix(&expected.0)
                        .is_some_and(|prefix| prefix.ends_with('.'))
            }
            _ => false,
        }
    }
    /// 参数必须是真实候选地址，不能是 FakeIP。
    pub fn matches_ip(&self, ip: IpAddr) -> bool {
        match self {
            Self::Ip(expected) => *expected == ip,
            Self::Cidr(network) => network.contains(ip),
            Self::Range(range) => range.contains(ip),
            _ => false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub target: RuleTarget,
    pub ports: PortSet,
    pub note: String,
}
impl RoutingRule {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.id.trim().is_empty() {
            return Err(ValidationError("规则 ID 不能为空"));
        }
        if self.name.trim().is_empty() {
            return Err(ValidationError("规则名称不能为空"));
        }
        self.target.validate()?;
        self.ports.validate()?;
        Ok(())
    }

    pub fn matches_domain(&self, domain: &DomainName, port: u16) -> bool {
        self.enabled && self.ports.contains(port) && self.target.matches_domain(domain)
    }
    pub fn matches_ip(&self, ip: IpAddr, port: u16) -> bool {
        self.enabled && self.ports.contains(port) && self.target.matches_ip(ip)
    }
}

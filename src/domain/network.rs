use serde::{Deserialize, Serialize};
use std::{fmt, net::IpAddr, str::FromStr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError(pub &'static str);
impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for ValidationError {}

/// 排序且合并相邻区间；空输入表示所有合法端口。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortSet(Vec<(u16, u16)>);
impl FromStr for PortSet {
    type Err = ValidationError;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.trim().is_empty() {
            return Ok(Self(vec![(1, u16::MAX)]));
        }
        fn port(s: &str) -> Result<u16, ValidationError> {
            let s = s.trim();
            if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
                return Err(ValidationError("端口必须为十进制整数"));
            }
            s.parse::<u16>()
                .ok()
                .filter(|v| *v != 0)
                .ok_or(ValidationError("端口必须位于 1–65535"))
        }
        let mut ranges = Vec::new();
        for item in input.split(',') {
            let (start, end) = match item.split_once('-') {
                Some((a, b)) => (port(a)?, port(b)?),
                None => {
                    let p = port(item)?;
                    (p, p)
                }
            };
            if start > end {
                return Err(ValidationError("端口范围起点不得大于终点"));
            }
            ranges.push((start, end));
        }
        ranges.sort_unstable();
        let mut merged: Vec<(u16, u16)> = Vec::new();
        for (start, end) in ranges {
            if let Some(last) = merged.last_mut()
                && u32::from(start) <= u32::from(last.1) + 1
            {
                last.1 = last.1.max(end);
                continue;
            }
            merged.push((start, end));
        }
        Ok(Self(merged))
    }
}
impl PortSet {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.0.is_empty() {
            return Err(ValidationError("端口集合不能为空"));
        }
        let mut previous_end = None;
        for &(start, end) in &self.0 {
            if start == 0 || start > end {
                return Err(ValidationError("端口区间无效"));
            }
            if previous_end.is_some_and(|previous| u32::from(start) <= u32::from(previous) + 1) {
                return Err(ValidationError("端口区间必须排序并规范化"));
            }
            previous_end = Some(end);
        }
        Ok(())
    }

    pub fn contains(&self, port: u16) -> bool {
        self.0
            .iter()
            .any(|&(start, end)| start <= port && port <= end)
    }
    pub fn intervals(&self) -> &[(u16, u16)] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpNetwork {
    address: IpAddr,
    prefix: u8,
}
fn bits(ip: IpAddr) -> (u128, u32) {
    match ip {
        IpAddr::V4(v) => (u32::from(v) as u128, 32),
        IpAddr::V6(v) => (u128::from(v), 128),
    }
}
fn address(value: u128, width: u32) -> IpAddr {
    if width == 32 {
        IpAddr::V4((value as u32).into())
    } else {
        IpAddr::V6(value.into())
    }
}
fn host_mask(host_bits: u32) -> u128 {
    if host_bits == 128 {
        u128::MAX
    } else {
        (1u128 << host_bits) - 1
    }
}
impl IpNetwork {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if Self::new(self.address, self.prefix)? != *self {
            return Err(ValidationError("CIDR 网络地址未规范化"));
        }
        Ok(())
    }

    pub fn new(ip: IpAddr, prefix: u8) -> Result<Self, ValidationError> {
        let (value, width) = bits(ip);
        if u32::from(prefix) > width {
            return Err(ValidationError("CIDR 前缀超出地址族范围"));
        }
        Ok(Self {
            address: address(value & !host_mask(width - u32::from(prefix)), width),
            prefix,
        })
    }
    pub fn address(&self) -> IpAddr {
        self.address
    }
    pub fn prefix(&self) -> u8 {
        self.prefix
    }
    pub fn contains(&self, ip: IpAddr) -> bool {
        let (value, width) = bits(ip);
        let (base, own_width) = bits(self.address);
        width == own_width && value & !host_mask(width - u32::from(self.prefix)) == base
    }
}
impl FromStr for IpNetwork {
    type Err = ValidationError;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let (ip, prefix) = input
            .trim()
            .split_once('/')
            .ok_or(ValidationError("CIDR 需要地址和前缀"))?;
        if prefix.is_empty() || !prefix.bytes().all(|c| c.is_ascii_digit()) {
            return Err(ValidationError("CIDR 前缀必须为整数"));
        }
        Self::new(
            ip.parse().map_err(|_| ValidationError("IP 地址无效"))?,
            prefix
                .parse()
                .map_err(|_| ValidationError("CIDR 前缀无效"))?,
        )
    }
}
impl fmt::Display for IpNetwork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpRange {
    start: IpAddr,
    end: IpAddr,
}
impl IpRange {
    pub fn validate(&self) -> Result<(), ValidationError> {
        Self::new(self.start, self.end).map(|_| ())
    }

    pub fn new(start: IpAddr, end: IpAddr) -> Result<Self, ValidationError> {
        let (a, aw) = bits(start);
        let (b, bw) = bits(end);
        if aw != bw {
            return Err(ValidationError("IP 范围不能混合地址族"));
        }
        if a > b {
            return Err(ValidationError("IP 范围起点不得大于终点"));
        }
        Ok(Self { start, end })
    }
    pub fn contains(&self, ip: IpAddr) -> bool {
        let (value, width) = bits(ip);
        let (start, own_width) = bits(self.start);
        width == own_width && start <= value && value <= bits(self.end).0
    }
    pub fn start(&self) -> IpAddr {
        self.start
    }
    pub fn end(&self) -> IpAddr {
        self.end
    }
    /// 最小 CIDR 覆盖，最多 2 * 地址位数个块，不逐地址展开。
    pub fn to_cidrs(&self) -> Vec<IpNetwork> {
        let (mut start, width) = bits(self.start);
        let end = bits(self.end).0;
        let mut result = Vec::new();
        loop {
            let mut host_bits = start.trailing_zeros().min(width);
            while host_mask(host_bits) > end - start {
                host_bits -= 1;
            }
            result.push(IpNetwork {
                address: address(start, width),
                prefix: (width - host_bits) as u8,
            });
            let last = start | host_mask(host_bits);
            if last == end {
                break;
            }
            start = last + 1;
        }
        result
    }
}
impl fmt::Display for IpRange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}-{}", self.start, self.end)
    }
}

use super::{DomainName, ValidationError};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, net::IpAddr};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProfileId(String);

impl ProfileId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        Uuid::parse_str(value)
            .map(|id| Self(id.hyphenated().to_string()))
            .map_err(|_| ValidationError("代理 ID 无效"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<(), ValidationError> {
        Self::parse(&self.0).map(|_| ())
    }
}

impl Default for ProfileId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRef(String);

impl CredentialRef {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        let value = value.trim();
        if value.is_empty()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(ValidationError("凭据引用无效"));
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<(), ValidationError> {
        Self::parse(&self.0).map(|_| ())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyProtocol {
    Socks5,
    Http,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyHost {
    Ip(IpAddr),
    Domain(DomainName),
}

impl ProxyHost {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        let value = value.trim();
        if value.is_empty() {
            return Err(ValidationError("代理地址不能为空"));
        }
        if let Ok(ip) = value.parse() {
            return Ok(Self::Ip(ip));
        }
        DomainName::parse(value).map(Self::Domain)
    }

    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Ip(_) => Ok(()),
            Self::Domain(domain) => domain.validate(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyProfile {
    pub id: ProfileId,
    pub name: String,
    pub protocol: ProxyProtocol,
    pub host: ProxyHost,
    pub port: u16,
    pub auth_enabled: bool,
    pub credential_ref: Option<CredentialRef>,
}

impl ProxyProfile {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.id.validate()?;
        if self.name.trim().is_empty() {
            return Err(ValidationError("代理名称不能为空"));
        }
        if self.port == 0 {
            return Err(ValidationError("端口必须位于 1–65535"));
        }
        if !self.auth_enabled && self.credential_ref.is_some() {
            return Err(ValidationError("未启用认证的代理不能引用凭据"));
        }
        self.host.validate()?;
        if let Some(reference) = &self.credential_ref {
            reference.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteProfileError {
    NotFound,
    ActiveProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteContext {
    Proxying,
    Direct,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyProfiles {
    profiles: BTreeMap<ProfileId, ProxyProfile>,
    active: Option<ProfileId>,
}

impl ProxyProfiles {
    pub fn create(&mut self, profile: ProxyProfile) -> Result<(), ValidationError> {
        profile.validate()?;
        if self.profiles.contains_key(&profile.id) {
            return Err(ValidationError("代理 ID 已存在"));
        }
        self.profiles.insert(profile.id.clone(), profile);
        Ok(())
    }

    pub fn update(&mut self, profile: ProxyProfile) -> Result<(), ValidationError> {
        profile.validate()?;
        let stored = self
            .profiles
            .get_mut(&profile.id)
            .ok_or(ValidationError("代理不存在"))?;
        *stored = profile;
        Ok(())
    }

    pub fn get(&self, id: &ProfileId) -> Option<&ProxyProfile> {
        self.profiles.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ProxyProfile> {
        self.profiles.values()
    }

    pub fn active(&self) -> Option<&ProxyProfile> {
        self.active.as_ref().and_then(|id| self.profiles.get(id))
    }

    pub fn require_active(&self) -> Result<&ProxyProfile, ValidationError> {
        let profile = self.active().ok_or(ValidationError("请先选择有效代理"))?;
        if profile.auth_enabled && profile.credential_ref.is_none() {
            return Err(ValidationError("当前代理需要补充认证凭据"));
        }
        Ok(profile)
    }

    pub fn active_id(&self) -> Option<&ProfileId> {
        self.active.as_ref()
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        for (id, profile) in &self.profiles {
            profile.validate()?;
            if id != &profile.id {
                return Err(ValidationError("代理索引与 ID 不一致"));
            }
        }
        if self
            .active
            .as_ref()
            .is_some_and(|id| !self.profiles.contains_key(id))
        {
            return Err(ValidationError("当前代理不存在"));
        }
        Ok(())
    }

    pub fn select(&mut self, id: &ProfileId) -> Result<(), ValidationError> {
        if !self.profiles.contains_key(id) {
            return Err(ValidationError("代理不存在"));
        }
        self.active = Some(id.clone());
        Ok(())
    }

    pub fn delete(
        &mut self,
        id: &ProfileId,
        context: DeleteContext,
    ) -> Result<ProxyProfile, DeleteProfileError> {
        if self.active.as_ref() == Some(id) && context == DeleteContext::Proxying {
            return Err(DeleteProfileError::ActiveProfile);
        }
        let deleted = self
            .profiles
            .remove(id)
            .ok_or(DeleteProfileError::NotFound)?;
        if self.active.as_ref() == Some(id) {
            self.active = None;
        }
        Ok(deleted)
    }
}

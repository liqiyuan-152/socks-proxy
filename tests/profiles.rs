use socks_proxy::domain::{
    CredentialRef, DeleteContext, DeleteProfileError, ProfileId, ProxyHost, ProxyProfile,
    ProxyProfiles, ProxyProtocol,
};

fn profile(name: &str) -> ProxyProfile {
    ProxyProfile {
        id: ProfileId::new(),
        name: name.to_owned(),
        protocol: ProxyProtocol::Socks5,
        host: ProxyHost::parse("proxy.example.com").unwrap(),
        port: 1080,
        auth_enabled: false,
        credential_ref: None,
    }
}

#[test]
fn crud_keeps_stable_ids_and_one_selection() {
    let mut profiles = ProxyProfiles::default();
    let first = profile("A");
    let first_id = first.id.clone();
    let second = profile("B");
    let second_id = second.id.clone();
    profiles.create(first).unwrap();
    profiles.create(second).unwrap();
    profiles.select(&first_id).unwrap();
    profiles.select(&second_id).unwrap();
    assert_eq!(profiles.active().unwrap().id, second_id);

    let mut edited = profiles.get(&first_id).unwrap().clone();
    edited.name = "A edited".into();
    edited.protocol = ProxyProtocol::Http;
    profiles.update(edited).unwrap();
    assert_eq!(profiles.get(&first_id).unwrap().id, first_id);
    assert_eq!(profiles.get(&first_id).unwrap().name, "A edited");
    assert_eq!(profiles.iter().count(), 2);
}

#[test]
fn authentication_is_represented_only_by_an_opaque_reference() {
    let mut profiles = ProxyProfiles::default();
    let mut authenticated = profile("authenticated");
    authenticated.auth_enabled = true;
    authenticated.credential_ref = Some(CredentialRef::parse("credential-version-7").unwrap());
    profiles.create(authenticated.clone()).unwrap();
    profiles.select(&authenticated.id).unwrap();
    assert_eq!(
        profiles
            .get(&authenticated.id)
            .unwrap()
            .credential_ref
            .as_ref()
            .unwrap()
            .as_str(),
        "credential-version-7"
    );

    authenticated.credential_ref = None;
    profiles.update(authenticated).unwrap();
    assert!(profiles.require_active().is_err());

    let mut inconsistent = profile("no auth");
    inconsistent.credential_ref = Some(CredentialRef::parse("unexpected-secret").unwrap());
    assert!(profiles.create(inconsistent).is_err());
}

#[test]
fn invalid_input_never_overwrites_a_profile() {
    for host in [
        "",
        "https://proxy.example",
        "proxy.example/path",
        "host:1080",
        "a b",
    ] {
        assert!(ProxyHost::parse(host).is_err(), "{host}");
    }
    assert!(ProfileId::parse("not-an-id").is_err());

    let mut profiles = ProxyProfiles::default();
    let original = profile("valid");
    let id = original.id.clone();
    profiles.create(original.clone()).unwrap();
    let mut invalid = original.clone();
    invalid.port = 0;
    assert!(profiles.update(invalid).is_err());
    assert_eq!(profiles.get(&id), Some(&original));
}

#[test]
fn active_profile_requires_switch_or_explicit_direct() {
    let mut profiles = ProxyProfiles::default();
    let first = profile("A");
    let first_id = first.id.clone();
    let second = profile("B");
    let second_id = second.id.clone();
    profiles.create(first).unwrap();
    profiles.create(second).unwrap();
    profiles.select(&first_id).unwrap();
    assert_eq!(
        profiles.delete(&first_id, DeleteContext::Proxying),
        Err(DeleteProfileError::ActiveProfile)
    );
    profiles.select(&second_id).unwrap();
    assert_eq!(
        profiles
            .delete(&first_id, DeleteContext::Proxying)
            .unwrap()
            .id,
        first_id
    );
    assert_eq!(profiles.active().unwrap().id, second_id);
    assert_eq!(
        profiles
            .delete(&second_id, DeleteContext::Direct)
            .unwrap()
            .id,
        second_id
    );
    assert!(profiles.active().is_none());
}

#[test]
fn missing_or_failed_selection_preserves_the_previous_selection() {
    let mut profiles = ProxyProfiles::default();
    assert!(profiles.require_active().is_err());
    let first = profile("A");
    let first_id = first.id.clone();
    profiles.create(first).unwrap();
    profiles.select(&first_id).unwrap();
    assert!(profiles.select(&ProfileId::new()).is_err());
    assert_eq!(profiles.require_active().unwrap().id, first_id);
}

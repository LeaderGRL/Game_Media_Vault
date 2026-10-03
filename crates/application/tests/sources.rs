mod support;

use game_media_vault_application::{
    ApiKey, ApplicationError, ConnectorPort, CredentialState, CredentialStorePort, ErrorKind,
    Machine, MachineSettingsPort, PortError, SourceDescription, clear_source_api_key,
    describe_sources, machine_registry, set_source_api_key, set_source_enabled,
};
use game_media_vault_domain::AssetType;
use support::{FakeConnector, FakeCredentials, FakeSettings};

/// A machine whose settings and credentials are kept in memory.
#[derive(Default)]
struct FakeMachine {
    settings: FakeSettings,
    credentials: FakeCredentials,
}

impl FakeMachine {
    fn machine(&self) -> Machine<'_> {
        Machine {
            settings: &self.settings,
            credentials: &self.credentials,
        }
    }
}

/// A Source that needs an API key.
fn keyed() -> FakeConnector {
    FakeConnector {
        source_id: "keyed-source",
        needs_api_key: true,
        ..FakeConnector::new(Vec::new())
    }
}

#[test]
fn sources_are_described_with_the_capabilities_planning_uses() {
    let boxes = FakeConnector::new(Vec::new());
    let snaps = FakeConnector {
        source_id: "snap-source",
        asset_types: vec![AssetType::Screenshot, AssetType::TitleScreen],
        ..FakeConnector::new(Vec::new())
    };
    let connectors: Vec<&dyn ConnectorPort> = vec![&boxes, &snaps];
    let machine = FakeMachine::default();

    let sources = describe_sources(&connectors, machine.machine()).unwrap();

    assert_eq!(
        sources,
        [
            SourceDescription {
                source_id: boxes.source_id().to_owned(),
                asset_types: vec![AssetType::BoxFront],
                direct_media_download: true,
                enabled: true,
                credential: CredentialState::NotNeeded,
                rate_limits: None,
            },
            SourceDescription {
                source_id: "snap-source".to_owned(),
                asset_types: vec![AssetType::Screenshot, AssetType::TitleScreen],
                direct_media_download: true,
                enabled: true,
                credential: CredentialState::NotNeeded,
                rate_limits: None,
            },
        ]
    );
    // Describing a Source never consults it.
    assert_eq!(*boxes.discover_calls.borrow(), 0);
}

#[test]
fn a_source_disabled_on_this_machine_is_described_as_such() {
    let boxes = FakeConnector::new(Vec::new());
    let connectors: Vec<&dyn ConnectorPort> = vec![&boxes];
    let machine = FakeMachine::default();
    machine
        .settings
        .set_source_enabled(boxes.source_id(), false)
        .unwrap();

    let sources = describe_sources(&connectors, machine.machine()).unwrap();

    assert!(!sources[0].enabled);
    assert_eq!(sources[0].asset_types, [AssetType::BoxFront]);
}

#[test]
fn sources_are_enabled_and_disabled_on_this_machine() {
    let boxes = FakeConnector::new(Vec::new());
    let connectors: Vec<&dyn ConnectorPort> = vec![&boxes];
    let machine = FakeMachine::default();

    let disabled =
        set_source_enabled(machine.machine(), &connectors, boxes.source_id(), false).unwrap();
    assert!(!disabled[0].enabled);
    assert_eq!(
        machine.settings.disabled_sources().unwrap(),
        [boxes.source_id()]
    );

    let enabled =
        set_source_enabled(machine.machine(), &connectors, boxes.source_id(), true).unwrap();
    assert!(enabled[0].enabled);
    assert!(machine.settings.disabled_sources().unwrap().is_empty());
}

#[test]
fn a_source_without_a_registered_connector_cannot_be_disabled() {
    let boxes = FakeConnector::new(Vec::new());
    let connectors: Vec<&dyn ConnectorPort> = vec![&boxes];
    let machine = FakeMachine::default();

    let error =
        set_source_enabled(machine.machine(), &connectors, "unknown-source", false).unwrap_err();

    assert_eq!(
        error,
        ApplicationError::SourceNotRegistered("unknown-source".to_owned())
    );
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(machine.settings.disabled_sources().unwrap().is_empty());
}

#[test]
fn an_api_key_never_shows_in_debug_output() {
    let key = ApiKey::new("secret-123").unwrap();

    assert!(!format!("{key:?}").contains("secret-123"));
    assert_eq!(key.expose(), "secret-123");
}

#[test]
fn a_blank_api_key_is_refused() {
    let error = ApiKey::new(" \t ").unwrap_err();

    assert_eq!(error, ApplicationError::InvalidApiKey);
    assert_eq!(error.kind(), ErrorKind::InvalidRequest);
}

#[test]
fn a_source_needing_an_api_key_says_whether_this_machine_stores_one() {
    let keyed = keyed();
    let connectors: Vec<&dyn ConnectorPort> = vec![&keyed];
    let machine = FakeMachine::default();
    let credential = |sources: &[SourceDescription]| sources[0].credential;

    let missing = describe_sources(&connectors, machine.machine()).unwrap();
    let stored = set_source_api_key(
        machine.machine(),
        &connectors,
        "keyed-source",
        &ApiKey::new("  key-of-the-user\n").unwrap(),
    )
    .unwrap();
    let kept = machine.credentials.keys.borrow().clone();
    let cleared = clear_source_api_key(machine.machine(), &connectors, "keyed-source").unwrap();

    assert_eq!(credential(&missing), CredentialState::Missing);
    assert_eq!(credential(&stored), CredentialState::Stored);
    // The key is kept trimmed, and only by the credential store.
    assert_eq!(kept["keyed-source"], "key-of-the-user");
    assert_eq!(credential(&cleared), CredentialState::Missing);
    assert!(machine.credentials.keys.borrow().is_empty());
}

#[test]
fn a_source_that_needs_no_api_key_is_given_none() {
    let boxes = FakeConnector::new(Vec::new());
    let connectors: Vec<&dyn ConnectorPort> = vec![&boxes];
    let machine = FakeMachine::default();

    let error = set_source_api_key(
        machine.machine(),
        &connectors,
        boxes.source_id(),
        &ApiKey::new("key").unwrap(),
    )
    .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::SourceNeedsNoApiKey(boxes.source_id().to_owned())
    );
    assert_eq!(error.kind(), ErrorKind::InvalidRequest);
    assert!(machine.credentials.keys.borrow().is_empty());
}

#[test]
fn an_unregistered_source_is_given_no_api_key() {
    let keyed = keyed();
    let connectors: Vec<&dyn ConnectorPort> = vec![&keyed];
    let machine = FakeMachine::default();

    let error = set_source_api_key(
        machine.machine(),
        &connectors,
        "unknown-source",
        &ApiKey::new("key").unwrap(),
    )
    .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::SourceNotRegistered("unknown-source".to_owned())
    );
    assert!(machine.credentials.keys.borrow().is_empty());
}

/// A credential store that cannot be read, as a machine without a running keyring service.
struct UnreadableCredentials;

impl CredentialStorePort for UnreadableCredentials {
    fn api_key(&self, _source_id: &str) -> Result<Option<ApiKey>, PortError> {
        Err(PortError::new("the credential store is locked".to_owned()))
    }

    fn set_api_key(&self, _source_id: &str, _key: &ApiKey) -> Result<(), PortError> {
        Err(PortError::new("the credential store is locked".to_owned()))
    }

    fn clear_api_key(&self, _source_id: &str) -> Result<(), PortError> {
        Err(PortError::new("the credential store is locked".to_owned()))
    }
}

#[test]
fn a_credential_store_that_cannot_be_read_still_lets_every_source_be_described() {
    let keyed = keyed();
    let boxes = FakeConnector::new(Vec::new());
    let connectors: Vec<&dyn ConnectorPort> = vec![&keyed, &boxes];
    let settings = FakeSettings::default();
    let machine = Machine {
        settings: &settings,
        credentials: &UnreadableCredentials,
    };

    let sources = describe_sources(&connectors, machine).unwrap();

    assert_eq!(sources[0].credential, CredentialState::Unreadable);
    assert_eq!(sources[1].credential, CredentialState::NotNeeded);
}

#[test]
fn what_a_source_is_known_to_limit_is_described_through_the_machine_registry() {
    let limited = FakeConnector {
        source_id: "limited-source",
        rate_limits: Some("one request a second"),
        ..FakeConnector::new(Vec::new())
    };
    let registry = machine_registry(vec![Box::new(limited) as Box<dyn ConnectorPort>], &[]);
    let connectors: Vec<&dyn ConnectorPort> = registry.iter().map(Box::as_ref).collect();
    let machine = FakeMachine::default();

    let sources = describe_sources(&connectors, machine.machine()).unwrap();

    assert_eq!(
        sources[0].rate_limits.as_deref(),
        Some("one request a second")
    );
}

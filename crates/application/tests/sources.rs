mod support;

use game_media_vault_application::{
    ApiKey, ApplicationError, ConnectorPort, CredentialField, CredentialState, CredentialStorePort,
    ErrorKind, Machine, MachineSettingsPort, PortError, SourceDescription, clear_source_api_key,
    clear_source_credential, describe_sources, machine_registry, set_source_api_key,
    set_source_credential, set_source_enabled,
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
                credential_fields: Vec::new(),
                rate_limits: None,
            },
            SourceDescription {
                source_id: "snap-source".to_owned(),
                asset_types: vec![AssetType::Screenshot, AssetType::TitleScreen],
                direct_media_download: true,
                enabled: true,
                credential: CredentialState::NotNeeded,
                credential_fields: Vec::new(),
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
fn a_blank_credential_or_one_with_control_characters_is_refused() {
    let blank = ApiKey::new(" \t ").unwrap_err();
    let controlled = ApiKey::new("pass\u{7}word").unwrap_err();

    assert_eq!(blank, ApplicationError::InvalidCredential);
    assert_eq!(controlled, ApplicationError::InvalidCredential);
    assert_eq!(blank.kind(), ErrorKind::InvalidRequest);
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

/// The credentials of a Source that needs an account: an identifier and a password for the
/// application, and an optional password for the user.
const ACCOUNT_FIELDS: &[CredentialField] = &[
    CredentialField {
        id: "dev-id",
        label: "Developer id",
        optional: false,
        single_word: true,
    },
    CredentialField {
        id: "dev-password",
        label: "Developer password",
        optional: false,
        single_word: false,
    },
    CredentialField {
        id: "user-password",
        label: "Account password",
        optional: true,
        single_word: false,
    },
];

fn account() -> FakeConnector {
    FakeConnector {
        source_id: "account-source",
        credential_fields: Some(ACCOUNT_FIELDS),
        ..FakeConnector::new(Vec::new())
    }
}

#[test]
fn a_source_asking_for_several_credentials_is_described_field_by_field() {
    let account = account();
    let connectors: Vec<&dyn ConnectorPort> = vec![&account];
    let machine = FakeMachine::default();
    let store = |field, value| {
        set_source_credential(
            machine.machine(),
            &connectors,
            "account-source",
            Some(field),
            &ApiKey::new(value).unwrap(),
        )
        .unwrap()
    };

    let partial = store("dev-id", "42");
    let complete = store("dev-password", "secret");

    let states = |sources: &[SourceDescription]| -> Vec<(String, CredentialState)> {
        sources[0]
            .credential_fields
            .iter()
            .map(|field| (field.id.clone(), field.state))
            .collect()
    };
    assert_eq!(partial[0].credential, CredentialState::Missing);
    assert_eq!(
        states(&partial),
        [
            ("dev-id".to_owned(), CredentialState::Stored),
            ("dev-password".to_owned(), CredentialState::Missing),
            ("user-password".to_owned(), CredentialState::Missing),
        ]
    );
    // An optional credential left out leaves the Source with what it needs.
    assert_eq!(complete[0].credential, CredentialState::Stored);
    assert_eq!(complete[0].credential_fields[2].label, "Account password");
    assert!(complete[0].credential_fields[2].optional);
    let kept = machine.credentials.keys.borrow().clone();
    assert_eq!(kept["account-source/dev-id"], "42");
    assert_eq!(kept["account-source/dev-password"], "secret");
}

#[test]
fn a_source_asking_for_several_credentials_is_given_each_by_name() {
    let account = account();
    let connectors: Vec<&dyn ConnectorPort> = vec![&account];
    let machine = FakeMachine::default();
    let key = ApiKey::new("value").unwrap();

    let unnamed =
        set_source_credential(machine.machine(), &connectors, "account-source", None, &key)
            .unwrap_err();
    let unknown = set_source_credential(
        machine.machine(),
        &connectors,
        "account-source",
        Some("token"),
        &key,
    )
    .unwrap_err();

    assert_eq!(
        unnamed,
        ApplicationError::CredentialFieldRequired {
            source_id: "account-source".to_owned(),
            fields: vec![
                "dev-id".to_owned(),
                "dev-password".to_owned(),
                "user-password".to_owned()
            ],
        }
    );
    assert_eq!(
        unknown,
        ApplicationError::UnknownCredentialField {
            source_id: "account-source".to_owned(),
            field: "token".to_owned(),
        }
    );
    assert_eq!(unnamed.kind(), ErrorKind::InvalidRequest);
    assert_eq!(unknown.kind(), ErrorKind::InvalidRequest);
    assert!(machine.credentials.keys.borrow().is_empty());
}

#[test]
fn a_sources_credentials_are_forgotten_one_or_all() {
    let account = account();
    let connectors: Vec<&dyn ConnectorPort> = vec![&account];
    let machine = FakeMachine::default();
    let store = |field| {
        set_source_credential(
            machine.machine(),
            &connectors,
            "account-source",
            Some(field),
            &ApiKey::new("value").unwrap(),
        )
        .unwrap();
    };
    store("dev-id");
    store("dev-password");

    clear_source_credential(
        machine.machine(),
        &connectors,
        "account-source",
        Some("dev-id"),
    )
    .unwrap();
    let one_left: Vec<String> = machine.credentials.keys.borrow().keys().cloned().collect();
    clear_source_credential(machine.machine(), &connectors, "account-source", None).unwrap();

    assert_eq!(one_left, ["account-source/dev-password"]);
    assert!(machine.credentials.keys.borrow().is_empty());
}

#[test]
fn a_password_may_hold_spaces_where_a_key_may_not() {
    let account = account();
    let connectors: Vec<&dyn ConnectorPort> = vec![&account];
    let machine = FakeMachine::default();
    let store = |field, value| {
        set_source_credential(
            machine.machine(),
            &connectors,
            "account-source",
            Some(field),
            &ApiKey::new(value).unwrap(),
        )
    };

    store("dev-password", "correct horse battery").unwrap();
    let spaced_id = store("dev-id", "4 2").unwrap_err();

    assert_eq!(
        machine.credentials.keys.borrow()["account-source/dev-password"],
        "correct horse battery"
    );
    assert_eq!(
        spaced_id,
        ApplicationError::CredentialNotOneWord("Developer id".to_owned())
    );
    assert_eq!(spaced_id.kind(), ErrorKind::InvalidRequest);
    assert!(
        !machine
            .credentials
            .keys
            .borrow()
            .contains_key("account-source/dev-id")
    );
}

#[test]
fn forgetting_a_sources_credentials_forgets_a_key_kept_under_its_own_name() {
    let account = account();
    let plain = FakeConnector::new(Vec::new());
    let connectors: Vec<&dyn ConnectorPort> = vec![&account, &plain];
    let machine = FakeMachine::default();
    // Keys an earlier version stored, when these Sources asked for an API key.
    for source_id in ["account-source", plain.source_id()] {
        machine
            .credentials
            .keys
            .borrow_mut()
            .insert(source_id.to_owned(), "earlier-key".to_owned());
    }

    clear_source_credential(machine.machine(), &connectors, "account-source", None).unwrap();
    clear_source_api_key(machine.machine(), &connectors, plain.source_id()).unwrap();

    assert!(machine.credentials.keys.borrow().is_empty());
}

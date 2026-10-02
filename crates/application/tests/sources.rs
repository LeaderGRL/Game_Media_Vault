mod support;

use std::sync::Mutex;

use game_media_vault_application::{
    ApplicationError, ConnectorPort, ErrorKind, MachineSettingsPort, PortError, SourceDescription,
    describe_sources, set_source_enabled,
};
use game_media_vault_domain::AssetType;
use support::FakeConnector;

/// Machine settings kept in memory.
#[derive(Default)]
struct FakeSettings {
    disabled: Mutex<Vec<String>>,
}

impl MachineSettingsPort for FakeSettings {
    fn disabled_sources(&self) -> Result<Vec<String>, PortError> {
        Ok(self.disabled.lock().unwrap().clone())
    }

    fn set_source_enabled(&self, source_id: &str, enabled: bool) -> Result<(), PortError> {
        let mut disabled = self.disabled.lock().unwrap();
        disabled.retain(|disabled| disabled != source_id);
        if !enabled {
            disabled.push(source_id.to_owned());
        }
        Ok(())
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

    let sources = describe_sources(&connectors, &[]);

    assert_eq!(
        sources,
        [
            SourceDescription {
                source_id: boxes.source_id().to_owned(),
                asset_types: vec![AssetType::BoxFront],
                direct_media_download: true,
                enabled: true,
            },
            SourceDescription {
                source_id: "snap-source".to_owned(),
                asset_types: vec![AssetType::Screenshot, AssetType::TitleScreen],
                direct_media_download: true,
                enabled: true,
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

    let sources = describe_sources(&connectors, &[boxes.source_id().to_owned()]);

    assert!(!sources[0].enabled);
    assert_eq!(sources[0].asset_types, [AssetType::BoxFront]);
}

#[test]
fn sources_are_enabled_and_disabled_on_this_machine() {
    let boxes = FakeConnector::new(Vec::new());
    let connectors: Vec<&dyn ConnectorPort> = vec![&boxes];
    let settings = FakeSettings::default();

    let disabled = set_source_enabled(&settings, &connectors, boxes.source_id(), false).unwrap();
    assert!(!disabled[0].enabled);
    assert_eq!(settings.disabled_sources().unwrap(), [boxes.source_id()]);

    let enabled = set_source_enabled(&settings, &connectors, boxes.source_id(), true).unwrap();
    assert!(enabled[0].enabled);
    assert!(settings.disabled_sources().unwrap().is_empty());
}

#[test]
fn a_source_without_a_registered_connector_cannot_be_disabled() {
    let boxes = FakeConnector::new(Vec::new());
    let connectors: Vec<&dyn ConnectorPort> = vec![&boxes];
    let settings = FakeSettings::default();

    let error = set_source_enabled(&settings, &connectors, "unknown-source", false).unwrap_err();

    assert_eq!(
        error,
        ApplicationError::SourceNotRegistered("unknown-source".to_owned())
    );
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(settings.disabled_sources().unwrap().is_empty());
}

use crate::ComputerConfig;

mod agent_stop;
mod authority;
mod capture;
mod fixture;
mod operator_stop;
mod read_limits;

#[test]
fn native_computer_config_stays_disabled_and_serializes_editable_defaults() {
    let config = ComputerConfig::default();
    assert!(!config.enabled);
    assert!(config.validate().is_ok());
    let serialized = serde_json::to_value(&config).unwrap();
    assert_eq!(serialized["enabled"], false);
    assert_eq!(serialized["maxWindows"], 64);
    assert_eq!(serialized["observationMaxAgeMs"], 10000);
    assert_eq!(
        serde_json::from_value::<ComputerConfig>(serialized).unwrap(),
        config
    );
    let mut invalid = config;
    invalid.action_timeout_ms = 0;
    assert!(invalid.validate().is_err());
}

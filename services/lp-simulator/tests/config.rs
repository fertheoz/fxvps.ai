#[test]
fn default_config_parses() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config/default.toml");
    let cfg = lp_simulator::SimConfig::load(path).unwrap();
    assert_eq!(cfg.instruments[0].security_id, "4001");
    assert_eq!(cfg.instruments[0].initial_mid.to_string(), "1.085");
    assert!(lp_simulator::SimConfig::from_toml("depth = 0").is_err());
}

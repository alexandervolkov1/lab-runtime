//! C9/C10 acceptance for the distinct managed-source reload operation.

use lab_core::{
    Query, QueryResult,
    managed::{ComponentId, ComponentState},
};
use lab_runtime::service::{ServiceHost, ServiceOptions};
use std::{fs, path::PathBuf};

fn temporary_path(label: &str, extension: &str) -> PathBuf {
    let mut entropy = [0u8; 12];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m8-{label}-{suffix}.{extension}"))
}

fn source(marker: &str) -> String {
    format!(
        r#"-- {marker}
return function(ctx)
  if ctx.phase == 'init' then return {{state={{}}, diagnostics={{}}}} end
  return {{status='ready', value=21.5, unit_id=ctx.unit_id, state={{}}, diagnostics={{}}}}
end
"#
    )
}

#[test]
fn c9_script_reload_changes_only_source_generation_and_resets_warmup() {
    let config = temporary_path("managed", "toml");
    let script = temporary_path("managed", "lua");
    fs::write(&script, source("generation one")).unwrap();
    let declared = script.to_string_lossy().replace('\\', "\\\\");
    let toml = format!(
        r#"schema_version=1
[runtime]
key="managed"
display_name="Managed"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[managed_components]]
id=201
instrument_id=201
key="source"
display_name="Configured source"
source="{declared}"
period_ms=100
"#
    );
    fs::write(&config, &toml).unwrap();
    let arg = config.to_string_lossy().into_owned();
    let mut service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &arg]).unwrap())
            .unwrap();
    let original_hash = service.loaded_configuration().unwrap().active().toml_hash();
    let QueryResult::Component(first) = service
        .owner()
        .query(Query::Component(ComponentId::new(201)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(first.generation, 1);

    fs::write(&script, source("generation two")).unwrap();
    service.reload_managed_scripts().unwrap();
    let QueryResult::Component(second) = service
        .owner()
        .query(Query::Component(ComponentId::new(201)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(second.generation, 2);
    assert!(matches!(
        second.state,
        ComponentState::Warming | ComponentState::Ready
    ));
    assert_eq!(
        service.loaded_configuration().unwrap().active().toml_hash(),
        original_hash
    );
    fs::remove_file(config).unwrap();
    fs::remove_file(script).unwrap();
}

#[test]
fn c10_invalid_script_preserves_committed_source_generation() {
    let config = temporary_path("managed-invalid", "toml");
    let script = temporary_path("managed-invalid", "lua");
    fs::write(&script, source("valid")).unwrap();
    let declared = script.to_string_lossy().replace('\\', "\\\\");
    let toml = format!(
        r#"schema_version=1
[runtime]
key="managed"
display_name="Managed"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[managed_components]]
id=201
instrument_id=201
key="source"
display_name="Configured source"
source="{declared}"
period_ms=100
"#
    );
    fs::write(&config, toml).unwrap();
    let arg = config.to_string_lossy().into_owned();
    let mut service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &arg]).unwrap())
            .unwrap();
    fs::write(&script, "this is not valid Lua (").unwrap();

    assert!(service.reload_managed_scripts().is_err());
    let QueryResult::Component(snapshot) = service
        .owner()
        .query(Query::Component(ComponentId::new(201)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(snapshot.generation, 1);
    fs::remove_file(config).unwrap();
    fs::remove_file(script).unwrap();
}

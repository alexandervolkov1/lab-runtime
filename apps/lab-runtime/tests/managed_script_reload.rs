//! C9/C10 acceptance for the distinct managed-source reload operation.

use lab_core::{
    Query, QueryResult,
    managed::{ComponentId, ComponentState},
};
use lab_runtime::host::Clock;
use lab_runtime::recorder::RecordingState;
use lab_runtime::service::{ServiceHost, ServiceOptions};
use std::{
    fs,
    path::PathBuf,
    sync::{Mutex, MutexGuard, OnceLock},
    time::{Duration, Instant},
};

fn lua_slots() -> MutexGuard<'static, ()> {
    static SLOTS: OnceLock<Mutex<()>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(())).lock().unwrap()
}

#[test]
fn c9_c11_source_reload_and_model_restart_commit_distinct_durable_activations() {
    let _slots = lua_slots();
    let config = temporary_path("managed-recorded", "toml");
    let script = temporary_path("managed-recorded", "lua");
    let database = temporary_path("managed-recorded", "sqlite");
    fs::write(&script, source("generation one")).unwrap();
    let declared = script.to_string_lossy().replace('\\', "\\\\");
    let database_path = database.to_string_lossy().replace('\\', "\\\\");
    let toml = format!(
        r#"schema_version=1
[runtime]
key="managed-recorded"
display_name="Managed recorded"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=true
path="{database_path}"
policy="required"
[[managed_components]]
id=201
instrument_id=201
key="source"
display_name="Configured source"
implementation="lua.v1"
source="{declared}"
period_ms=100
[[managed_components]]
id=202
instrument_id=202
key="moving_mean"
display_name="Native moving mean"
implementation="native.moving_mean.v1"
input_instrument_id=201
period_ms=100
config={{window=3}}
"#
    );
    fs::write(&config, &toml).unwrap();
    let arg = config.to_string_lossy().into_owned();
    let mut service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &arg]).unwrap())
            .unwrap();
    let original_hash = service.loaded_configuration().unwrap().active().toml_hash();
    let now = service.clock().now();
    service
        .owner_mut()
        .start_recording("managed lifecycle", now)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording {
        assert!(Instant::now() < deadline);
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }

    fs::write(&script, source("generation two")).unwrap();
    service.reload_managed_sources().unwrap();
    assert_eq!(
        service
            .owner()
            .recording_status()
            .unwrap()
            .activation_generation,
        2
    );
    let QueryResult::Component(native_after_source_reload) = service
        .owner()
        .query(Query::Component(ComponentId::new(202)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(native_after_source_reload.generation, 2);
    assert_eq!(
        service.loaded_configuration().unwrap().active().toml_hash(),
        original_hash
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while service
        .owner()
        .recording_status()
        .is_some_and(|status| status.outstanding_groups != 0)
    {
        assert!(Instant::now() < deadline);
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    fs::write(&script, "this mutable path is no longer valid Lua").unwrap();
    let restarted = service.restart_models();
    assert_eq!(
        restarted,
        Ok(lab_runtime::service::RestartModelsResult {
            models: 2,
            generation: 3,
        }),
        "recording status: {:?}",
        service.owner().recording_status()
    );
    assert_eq!(
        service
            .owner()
            .recording_status()
            .unwrap()
            .activation_generation,
        3
    );
    let QueryResult::Component(component) = service
        .owner()
        .query(Query::Component(ComponentId::new(201)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(component.generation, 3);
    let QueryResult::Component(native) = service
        .owner()
        .query(Query::Component(ComponentId::new(202)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(native.generation, 3);
    assert_eq!(native.implementation.as_str(), "native.moving_mean.v1");
    assert_eq!(
        service.loaded_configuration().unwrap().active().toml_hash(),
        original_hash
    );

    service.request_shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    while service.shutdown_step().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    drop(service);
    let connection = rusqlite::Connection::open(&database).unwrap();
    let lifecycle_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM records WHERE kind='configuration_lifecycle'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(lifecycle_count, 2);
    drop(connection);
    fs::remove_file(config).unwrap();
    fs::remove_file(script).unwrap();
    fs::remove_file(database).unwrap();
}

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
fn c9_source_reload_changes_only_source_generation_and_resets_warmup() {
    let _slots = lua_slots();
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
implementation="lua.v1"
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
    fs::write(
        &config,
        toml.replace(
            "display_name=\"Managed\"",
            "display_name=\"Managed renamed\"",
        ),
    )
    .unwrap();
    service.reload_configuration().unwrap();
    let active = service.loaded_configuration().unwrap().active();
    assert_ne!(active.toml_hash(), original_hash);
    assert!(
        active
            .artifacts()
            .iter()
            .any(|artifact| artifact.bytes() == source("generation one").as_bytes())
    );
    let QueryResult::Component(before_explicit_reload) = service
        .owner()
        .query(Query::Component(ComponentId::new(201)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(before_explicit_reload.generation, 1);
    let configuration_hash = active.toml_hash();

    service.reload_managed_sources().unwrap();
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
        configuration_hash
    );
    fs::remove_file(config).unwrap();
    fs::remove_file(script).unwrap();
}

#[test]
fn c10_invalid_script_preserves_committed_source_generation() {
    let _slots = lua_slots();
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
implementation="lua.v1"
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

    assert!(service.reload_managed_sources().is_err());
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

#[test]
fn c9_multi_component_reload_prepares_all_before_one_generation_commit() {
    let _slots = lua_slots();
    let config = temporary_path("managed-batch", "toml");
    let first_script = temporary_path("managed-first", "lua");
    let second_script = temporary_path("managed-second", "lua");
    fs::write(&first_script, source("first generation one")).unwrap();
    fs::write(&second_script, source("second generation one")).unwrap();
    let first = first_script.to_string_lossy().replace('\\', "\\\\");
    let second = second_script.to_string_lossy().replace('\\', "\\\\");
    let toml = format!(
        r#"schema_version=1
[runtime]
key="managed-batch"
display_name="Managed batch"
[server]
host="127.0.0.1"
port=0
[recording]
enabled=false
policy="best_effort"
[[managed_components]]
id=201
instrument_id=201
key="first"
display_name="First source"
implementation="lua.v1"
source="{first}"
period_ms=100
[[managed_components]]
id=202
instrument_id=202
key="second"
display_name="Second source"
implementation="lua.v1"
source="{second}"
period_ms=100
"#
    );
    fs::write(&config, toml).unwrap();
    let arg = config.to_string_lossy().into_owned();
    let mut service =
        ServiceHost::startup(ServiceOptions::parse(&["--serve", "--config", &arg]).unwrap())
            .unwrap();

    fs::write(&first_script, source("first candidate")).unwrap();
    fs::write(&second_script, "invalid Lua candidate (").unwrap();
    assert!(service.reload_managed_sources().is_err());
    for id in [201, 202] {
        let QueryResult::Component(snapshot) = service
            .owner()
            .query(Query::Component(ComponentId::new(id)))
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(snapshot.generation, 1, "component {id} changed partially");
    }

    fs::write(&second_script, source("second candidate")).unwrap();
    service.reload_managed_sources().unwrap();
    for id in [201, 202] {
        let QueryResult::Component(snapshot) = service
            .owner()
            .query(Query::Component(ComponentId::new(id)))
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(snapshot.generation, 2);
    }
    fs::remove_file(config).unwrap();
    fs::remove_file(first_script).unwrap();
    fs::remove_file(second_script).unwrap();
}

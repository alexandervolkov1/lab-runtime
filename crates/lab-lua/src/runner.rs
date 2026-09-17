//! One disposable Lua 5.4 VM with a capability allowlist and independently checked budgets.

use lab_core::managed::{
    ComponentError, ComponentResult, ComponentStatus, Invocation, InvocationPhase,
    MAX_IMPLEMENTATION_TEXT_BYTES, PlainData, PlainValue,
};
use mlua::{ChunkMode, Function, HookTriggers, Lua, LuaOptions, StdLib, Table, Value, VmState};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};

const HEAP_LIMIT: usize = 8 * 1024 * 1024;
const INSTRUCTION_LIMIT: usize = 100_000;
const HOOK_INTERVAL: usize = 100;
const HOST_CALL_LIMIT: usize = 128;
const RESULT_LIMIT: usize = 8192;
const RESULT_LEAVES: usize = 512;

fn budget(deadline: Instant, cancelled: &AtomicBool) -> mlua::Result<()> {
    if cancelled.load(Ordering::Relaxed) || Instant::now() >= deadline {
        return Err(mlua::Error::RuntimeError("__lab_deadline__".into()));
    }
    Ok(())
}

fn map_error(error: &mlua::Error) -> ComponentError {
    match error {
        mlua::Error::MemoryError(_) => ComponentError::MemoryLimit,
        mlua::Error::SyntaxError { .. } => ComponentError::InvalidImplementation,
        mlua::Error::RuntimeError(text) if text.starts_with("__lab_deadline__") => {
            ComponentError::Deadline
        }
        mlua::Error::RuntimeError(text) if text.starts_with("__lab_instruction__") => {
            ComponentError::ExecutionLimit
        }
        mlua::Error::RuntimeError(text) if text.starts_with("__lab_calls__") => {
            ComponentError::AdapterCallLimit
        }
        mlua::Error::CallbackError { cause, .. } => map_error(cause),
        _ => ComponentError::Executor,
    }
}

fn counted(calls: &AtomicUsize, deadline: Instant, cancelled: &AtomicBool) -> mlua::Result<()> {
    budget(deadline, cancelled)?;
    let previous = calls.fetch_add(1, Ordering::Relaxed);
    if previous >= HOST_CALL_LIMIT {
        return Err(mlua::Error::RuntimeError("__lab_calls__".into()));
    }
    Ok(())
}

fn scalar(value: Value) -> mlua::Result<f64> {
    let number = match value {
        Value::Number(number) => number,
        Value::Integer(number) if number.unsigned_abs() <= (1_u64 << 53) => number as f64,
        _ => return Err(mlua::Error::RuntimeError("invalid numeric argument".into())),
    };
    if !number.is_finite() {
        return Err(mlua::Error::RuntimeError(
            "nonfinite numeric argument".into(),
        ));
    }
    Ok(number)
}

/// The trusted arity gate keeps a private base `select` upvalue unreachable from
/// the guest environment. Only at most two fixed arguments reach Rust conversion;
/// nil extra arguments are counted, and no variadic Rust vector is materialized.
fn gate(lua: &Lua, guest: &Table, name: &str, arity: usize, host: Function) -> mlua::Result<()> {
    let private = lua.create_table()?;
    let select: Function = lua.globals().raw_get("select")?;
    private.raw_set("select", select)?;
    private.raw_set("host", host)?;
    let reject = lua.create_function(|_, ()| -> mlua::Result<()> {
        Err(mlua::Error::RuntimeError("invalid host arity".into()))
    })?;
    private.raw_set("reject", reject)?;
    private.raw_set("required", arity)?;
    let wrapper: Function = lua.load(
        "return function(...) if select('#', ...) ~= required then reject() end return host(...) end"
    ).set_environment(private).set_mode(ChunkMode::Text).eval()?;
    guest.raw_set(name, wrapper)
}

fn environment(
    lua: &Lua,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
) -> mlua::Result<Table> {
    let guest = lua.create_table()?;
    let type_calls = calls.clone();
    let type_cancel = cancelled.clone();
    let classifier = lua.create_function(move |_, value: Value| {
        counted(&type_calls, deadline, &type_cancel)?;
        Ok(match value {
            Value::Nil => "nil",
            Value::Boolean(_) => "boolean",
            Value::Integer(_) | Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Table(_) => "table",
            Value::Function(_) => "function",
            Value::Thread(_) => "thread",
            Value::UserData(_) | Value::LightUserData(_) => "userdata",
            _ => "unknown",
        })
    })?;
    gate(lua, &guest, "type", 1, classifier)?;
    let error_calls = calls.clone();
    let error_cancel = cancelled.clone();
    let error = lua.create_function(move |_, value: Value| -> mlua::Result<()> {
        counted(&error_calls, deadline, &error_cancel)?;
        let Value::String(message) = value else {
            return Err(mlua::Error::RuntimeError(
                "error accepts bounded text".into(),
            ));
        };
        if message.as_bytes().len() > 256 {
            return Err(mlua::Error::RuntimeError("oversized error message".into()));
        }
        let text = message
            .to_str()
            .map_err(|_| mlua::Error::RuntimeError("non-UTF8 error".into()))?;
        Err(mlua::Error::RuntimeError(text.to_string()))
    })?;
    gate(lua, &guest, "error", 1, error)?;

    let math = lua.create_table()?;
    for (name, operation) in [
        ("abs", f64::abs as fn(f64) -> f64),
        ("floor", f64::floor),
        ("ceil", f64::ceil),
        ("sqrt", f64::sqrt),
        ("exp", f64::exp),
        ("log", f64::ln),
        ("sin", f64::sin),
        ("cos", f64::cos),
    ] {
        let math_calls = calls.clone();
        let math_cancel = cancelled.clone();
        let host = lua.create_function(move |_, argument: Value| {
            counted(&math_calls, deadline, &math_cancel)?;
            let value = operation(scalar(argument)?);
            if !value.is_finite() {
                return Err(mlua::Error::RuntimeError("nonfinite math result".into()));
            }
            Ok(value)
        })?;
        gate(lua, &math, name, 1, host)?;
    }
    for (name, operation) in [("min", f64::min as fn(f64, f64) -> f64), ("max", f64::max)] {
        let math_calls = calls.clone();
        let math_cancel = cancelled.clone();
        let host = lua.create_function(move |_, (left, right): (Value, Value)| {
            counted(&math_calls, deadline, &math_cancel)?;
            let result = operation(scalar(left)?, scalar(right)?);
            if !result.is_finite() {
                return Err(mlua::Error::RuntimeError("nonfinite math result".into()));
            }
            Ok(result)
        })?;
        gate(lua, &math, name, 2, host)?;
    }
    math.raw_set("pi", std::f64::consts::PI)?;
    guest.raw_set("math", math)?;
    Ok(guest)
}

fn encode_data(lua: &Lua, data: &PlainData) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (key, value) in &data.fields {
        match value {
            PlainValue::Number(number) => table.raw_set(key.as_str(), *number)?,
            PlainValue::Boolean(boolean) => table.raw_set(key.as_str(), *boolean)?,
            PlainValue::Text(text) => table.raw_set(key.as_str(), text.as_str())?,
            PlainValue::Numbers(numbers) => {
                let array = lua.create_table()?;
                for (index, number) in numbers.iter().enumerate() {
                    array.raw_set(index + 1, *number)?;
                }
                table.raw_set(key.as_str(), array)?;
            }
        }
    }
    Ok(table)
}

fn context(lua: &Lua, job: &Invocation) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.raw_set(
        "phase",
        if job.phase == InvocationPhase::Init {
            "init"
        } else {
            "step"
        },
    )?;
    table.raw_set("config", encode_data(lua, &job.definition.config)?)?;
    table.raw_set("state", encode_data(lua, &job.state)?)?;
    table.raw_set("unit_id", job.definition.manifest.unit.id())?;
    table.raw_set("now_seconds", job.at.as_secs_f64())?;
    table.raw_set("dt_seconds", job.dt.as_secs_f64())?;
    if let Some(input) = job.input {
        let nested = lua.create_table()?;
        nested.raw_set("value", input.value)?;
        nested.raw_set("unit_id", input.unit.id())?;
        nested.raw_set("quality", "good")?;
        nested.raw_set("observation_seconds", input.freshness_at.as_secs_f64())?;
        table.raw_set("input", nested)?;
    }
    Ok(table)
}

struct Limits {
    bytes: usize,
    leaves: usize,
}

impl Limits {
    fn add(&mut self, bytes: usize, leaves: usize) -> Result<(), ComponentError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or(ComponentError::DataLimit)?;
        self.leaves = self
            .leaves
            .checked_add(leaves)
            .ok_or(ComponentError::DataLimit)?;
        if self.bytes > RESULT_LIMIT || self.leaves > RESULT_LEAVES {
            return Err(ComponentError::DataLimit);
        }
        Ok(())
    }
}

fn decode_array(table: Table, limits: &mut Limits) -> Result<Vec<f64>, ComponentError> {
    if table.metatable().is_some() {
        return Err(ComponentError::InvalidResult);
    }
    let mut values = [None; 64];
    let mut count = 0usize;
    let mut highest = 0usize;
    for entry in table.pairs::<Value, Value>() {
        let (key, value) = entry.map_err(|_| ComponentError::InvalidResult)?;
        let Value::Integer(index) = key else {
            return Err(ComponentError::InvalidResult);
        };
        if !(1..=64).contains(&index) {
            return Err(ComponentError::DataLimit);
        }
        let index = index as usize;
        if values[index - 1].is_some() {
            return Err(ComponentError::InvalidResult);
        }
        values[index - 1] = Some(scalar(value).map_err(|_| ComponentError::InvalidResult)?);
        highest = highest.max(index);
        count += 1;
        limits.add(8, 1)?;
        if count > 64 {
            return Err(ComponentError::DataLimit);
        }
    }
    if count != highest {
        return Err(ComponentError::InvalidResult);
    }
    Ok(values
        .into_iter()
        .take(count)
        .map(|value| value.expect("dense keys checked"))
        .collect())
}

fn decode_data(table: Table, limits: &mut Limits) -> Result<PlainData, ComponentError> {
    if table.metatable().is_some() {
        return Err(ComponentError::InvalidResult);
    }
    let mut fields = BTreeMap::new();
    for entry in table.pairs::<Value, Value>() {
        let (key, value) = entry.map_err(|_| ComponentError::InvalidResult)?;
        if fields.len() >= 16 {
            return Err(ComponentError::DataLimit);
        }
        let Value::String(key) = key else {
            return Err(ComponentError::InvalidResult);
        };
        if key.as_bytes().is_empty() || key.as_bytes().len() > 32 {
            return Err(ComponentError::DataLimit);
        }
        let key = key.to_str().map_err(|_| ComponentError::InvalidResult)?;
        if !key.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(ComponentError::InvalidResult);
        }
        limits.add(key.len() + 1, 0)?;
        let value = match value {
            Value::Boolean(boolean) => {
                limits.add(2, 1)?;
                PlainValue::Boolean(boolean)
            }
            Value::Number(_) | Value::Integer(_) => {
                let number = scalar(value).map_err(|_| ComponentError::InvalidResult)?;
                limits.add(9, 1)?;
                PlainValue::Number(number)
            }
            Value::String(text) => {
                if text.as_bytes().len() > 128 {
                    return Err(ComponentError::DataLimit);
                }
                let text = text.to_str().map_err(|_| ComponentError::InvalidResult)?;
                limits.add(3 + text.len(), 1)?;
                PlainValue::Text(text.to_string())
            }
            Value::Table(array) => PlainValue::Numbers(decode_array(array, limits)?),
            _ => return Err(ComponentError::InvalidResult),
        };
        fields.insert(key.to_string(), value);
    }
    let result = PlainData { fields };
    result.validate()?;
    Ok(result)
}

fn diagnostics(table: Table, limits: &mut Limits) -> Result<Vec<String>, ComponentError> {
    if table.metatable().is_some() {
        return Err(ComponentError::InvalidResult);
    }
    let mut entries = [None, None, None, None];
    let mut count = 0;
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|_| ComponentError::InvalidResult)?;
        let Value::Integer(index) = key else {
            return Err(ComponentError::InvalidResult);
        };
        if !(1..=4).contains(&index) {
            return Err(ComponentError::DataLimit);
        }
        let Value::String(text) = value else {
            return Err(ComponentError::InvalidResult);
        };
        if text.as_bytes().len() > 256 {
            return Err(ComponentError::DataLimit);
        }
        let text = text.to_str().map_err(|_| ComponentError::InvalidResult)?;
        limits.add(text.len() + 3, 1)?;
        entries[index as usize - 1] = Some(text.to_string());
        count += 1;
    }
    if entries.iter().take(count).any(Option::is_none) {
        return Err(ComponentError::InvalidResult);
    }
    Ok(entries
        .into_iter()
        .take(count)
        .map(|entry| entry.expect("dense keys checked"))
        .collect())
}

fn decode_result(table: Table, job: &Invocation) -> Result<ComponentResult, ComponentError> {
    if table.metatable().is_some() {
        return Err(ComponentError::InvalidResult);
    }
    let mut limits = Limits {
        bytes: 0,
        leaves: 0,
    };
    let mut status = None;
    let mut value = None;
    let mut unit = None;
    let mut state = None;
    let mut messages = None;
    let mut count = 0usize;
    for entry in table.pairs::<Value, Value>() {
        let (key, item) = entry.map_err(|_| ComponentError::InvalidResult)?;
        count += 1;
        if count > 5 {
            return Err(ComponentError::InvalidResult);
        }
        let Value::String(key) = key else {
            return Err(ComponentError::InvalidResult);
        };
        if key.as_bytes().len() > 32 {
            return Err(ComponentError::DataLimit);
        }
        let key = key.to_str().map_err(|_| ComponentError::InvalidResult)?;
        limits.add(key.len() + 1, 0)?;
        match key.as_ref() {
            "status" if job.phase == InvocationPhase::Step => {
                let Value::String(text) = item else {
                    return Err(ComponentError::InvalidResult);
                };
                let text = text.to_str().map_err(|_| ComponentError::InvalidResult)?;
                limits.add(text.len() + 3, 1)?;
                status = Some(match text.as_ref() {
                    "warming" => ComponentStatus::Warming,
                    "ready" => ComponentStatus::Ready,
                    "unavailable" => ComponentStatus::Unavailable,
                    _ => return Err(ComponentError::InvalidResult),
                });
            }
            "value" if job.phase == InvocationPhase::Step => {
                limits.add(9, 1)?;
                value = Some(scalar(item).map_err(|_| ComponentError::InvalidResult)?);
            }
            "unit_id" if job.phase == InvocationPhase::Step => {
                let Value::String(text) = item else {
                    return Err(ComponentError::InvalidResult);
                };
                if text.as_bytes().len() > 32 {
                    return Err(ComponentError::DataLimit);
                }
                let text = text.to_str().map_err(|_| ComponentError::InvalidResult)?;
                limits.add(text.len() + 3, 1)?;
                if text.as_ref() != job.definition.manifest.unit.id() {
                    return Err(ComponentError::InvalidResult);
                }
                unit = Some(job.definition.manifest.unit);
            }
            "state" => {
                let Value::Table(map) = item else {
                    return Err(ComponentError::InvalidResult);
                };
                state = Some(decode_data(map, &mut limits)?);
            }
            "diagnostics" => {
                let Value::Table(array) = item else {
                    return Err(ComponentError::InvalidResult);
                };
                messages = Some(diagnostics(array, &mut limits)?);
            }
            _ => return Err(ComponentError::InvalidResult),
        }
    }
    let state = state.ok_or(ComponentError::InvalidResult)?;
    let diagnostics = messages.ok_or(ComponentError::InvalidResult)?;
    if job.phase == InvocationPhase::Init {
        if count != 2 {
            return Err(ComponentError::InvalidResult);
        }
        return Ok(ComponentResult {
            status: ComponentStatus::Init,
            value: None,
            unit: job.definition.manifest.unit,
            state,
            diagnostics,
        });
    }
    let status = status.ok_or(ComponentError::InvalidResult)?;
    let unit = unit.ok_or(ComponentError::InvalidResult)?;
    if (status == ComponentStatus::Ready) != value.is_some() {
        return Err(ComponentError::InvalidResult);
    }
    Ok(ComponentResult {
        status,
        value,
        unit,
        state,
        diagnostics,
    })
}

fn evaluate(
    lua: &Lua,
    job: &Invocation,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
) -> Result<ComponentResult, ComponentError> {
    let calls = Arc::new(AtomicUsize::new(0));
    let guest =
        environment(lua, deadline, cancelled.clone(), calls).map_err(|error| map_error(&error))?;
    budget(deadline, &cancelled).map_err(|error| map_error(&error))?;
    let callback: Function = lua
        .load(source(job)?)
        .set_mode(ChunkMode::Text)
        .set_environment(guest)
        .eval()
        .map_err(|error| map_error(&error))?;
    let context = context(lua, job).map_err(|error| map_error(&error))?;
    let table: Table = callback.call(context).map_err(|error| map_error(&error))?;
    budget(deadline, &cancelled).map_err(|error| map_error(&error))?;
    decode_result(table, job)
}

fn source(job: &Invocation) -> Result<&str, ComponentError> {
    if job.definition.implementation.id().as_str() != crate::IMPLEMENTATION_ID {
        return Err(ComponentError::InvalidConfiguration);
    }
    job.definition
        .implementation
        .artifact()
        .text()
        .ok_or(ComponentError::InvalidConfiguration)
}

/// Build, run, convert and dispose one fresh VM within the host's acceptance deadline.
/// A stalled VM may outlive that deadline; only a fixed supervisor can quarantine it.
pub fn run_bounded(
    job: &Invocation,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
) -> Result<ComponentResult, ComponentError> {
    let source = source(job)?;
    if source.len() > MAX_IMPLEMENTATION_TEXT_BYTES || source.is_empty() {
        return Err(ComponentError::DataLimit);
    }
    job.definition.config.validate()?;
    job.state.validate()?;
    if job.at.as_secs_f64().is_infinite()
        || !job.dt.as_secs_f64().is_finite()
        || job.input.is_some_and(|input| !input.value.is_finite())
    {
        return Err(ComponentError::InvalidConfiguration);
    }
    budget(deadline, &cancelled).map_err(|error| map_error(&error))?;
    let lua =
        Lua::new_with(StdLib::NONE, LuaOptions::default()).map_err(|error| map_error(&error))?;
    lua.set_memory_limit(HEAP_LIMIT)
        .map_err(|error| map_error(&error))?;
    if lua.used_memory() > HEAP_LIMIT {
        return Err(ComponentError::DataLimit);
    }
    let hooks = Arc::new(AtomicUsize::new(0));
    let counter = hooks.clone();
    let hook_cancel = cancelled.clone();
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(HOOK_INTERVAL as u32),
        move |_, _| {
            budget(deadline, &hook_cancel)?;
            let count = counter.fetch_add(HOOK_INTERVAL, Ordering::Relaxed) + HOOK_INTERVAL;
            if count > INSTRUCTION_LIMIT {
                return Err(mlua::Error::RuntimeError("__lab_instruction__".into()));
            }
            Ok(VmState::Continue)
        },
    )
    .map_err(|error| map_error(&error))?;
    let outcome = evaluate(&lua, job, deadline, cancelled.clone());
    drop(lua); // Lua GC and destruction belong to this worker, never Runtime's safety lane.
    budget(deadline, &cancelled).map_err(|error| map_error(&error))?;
    outcome
}

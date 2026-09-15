//! Two bounded observation-only Lua proofs selected by the trusted host.
//! These scripts have no transport, actuator, safety or evidence callback.

/// A finite virtual temperature model with trusted baseline/rate configuration.
/// It uses the host's elapsed time; a script-returned timestamp is never accepted.
pub const VIRTUAL_MODEL_SOURCE: &str = r#"
return function(ctx)
  if ctx.phase == 'init' then
    return {state={}, diagnostics={}}
  end
  local baseline = ctx.config.baseline
  local rate = ctx.config.rate
  local next_value = baseline + rate * ctx.now_seconds
  return {
    status='ready', value=next_value, unit_id=ctx.unit_id,
    state={last=next_value}, diagnostics={}
  }
end
"#;

/// One-input three-sample moving mean with a dense finite state vector.
/// Warming returns no value, so no native controller can arm from partial state.
pub const MOVING_MEAN_SOURCE: &str = r#"
return function(ctx)
  if ctx.phase == 'init' then
    return {state={values={}}, diagnostics={}}
  end
  local previous = ctx.state.values or {}
  local values = {}
  local first = 1
  if #previous == 3 then first = 2 end
  for i=first,#previous do values[#values+1] = previous[i] end
  values[#values+1] = ctx.input.value
  if #values < 3 then
    return {status='warming', unit_id=ctx.unit_id,
      state={values=values}, diagnostics={}}
  end
  local mean = (values[1] + values[2] + values[3]) / 3
  return {status='ready', value=mean, unit_id=ctx.unit_id,
    state={values=values}, diagnostics={}}
end
"#;

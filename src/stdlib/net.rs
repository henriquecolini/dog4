//! HTTP builtins (the `net` feature and the `Net` capability).

use super::{Library, checked_str, num, text};
use crate::error::{Error, ErrorKind, Result};
use crate::value::{Value, to_json};
use crate::vm::Vm;

const DEFAULT_TIMEOUT: u64 = 10;

/// The request timeout in whole seconds (minreq's unit): the script's own,
/// clamped to 1–60 s, but no longer than the run has left (rounded up).
fn timeout(vm: &mut Vm, value: Option<&Value>) -> Result<u64> {
    // Stop before connecting if the run is already out of time or interrupted.
    vm.meter().check_now()?;
    let secs = value.map_or(DEFAULT_TIMEOUT, |v| num(v).clamp(1.0, 60.0) as u64);
    Ok(match vm.meter().remaining() {
        Some(left) => secs.min(left.as_secs() + u64::from(left.subsec_nanos() > 0)),
        None => secs,
    })
}

fn send(vm: &mut Vm, request: minreq::Request) -> Result<Value> {
    let body = receive(request, vm.meter().max_str);
    // The VM only looks at the clock every few thousand fuel units, and a request
    // costs almost none, so check here: a request cut short by the run's deadline
    // reports the timeout, and a loop of requests can't outlive it.
    vm.meter().check_now()?;
    checked_str(vm, body?)
}

fn receive(request: minreq::Request, max: usize) -> Result<String> {
    let failed = |e: minreq::Error| Error::new(ErrorKind::Io, format!("request failed: {e}"));
    let response = request.send_lazy().map_err(failed)?;
    if !(200..300).contains(&response.status_code) {
        return Err(Error::new(
            ErrorKind::Io,
            format!(
                "request failed with status {} {}",
                response.status_code, response.reason_phrase
            ),
        ));
    }
    let mut body = Vec::new();
    for byte in response {
        let (byte, _) = byte.map_err(failed)?;
        if body.len() >= max {
            return Err(Error::limit(crate::error::Limit::StrSize));
        }
        body.push(byte);
    }
    String::from_utf8(body).map_err(|_| Error::new(ErrorKind::Io, "the response is not UTF-8 text"))
}

fn body_text(value: &Value) -> Result<String> {
    Ok(match value {
        Value::Str(s) => s.to_string(),
        other => to_json(other)?.to_string(),
    })
}

fn get(vm: &mut Vm, args: &mut [Value]) -> Result<Value> {
    let request = minreq::get(text(&args[0])).with_timeout(timeout(vm, args.get(1))?);
    send(vm, request)
}

fn post(vm: &mut Vm, args: &mut [Value]) -> Result<Value> {
    let mut request = minreq::post(text(&args[0])).with_timeout(timeout(vm, args.get(2))?);
    if let Some(body) = args.get(1) {
        request = request.with_body(body_text(body)?);
    }
    send(vm, request)
}

pub(super) fn install(lib: &mut Library) {
    lib.def("get", "url: str", get);
    lib.def("get", "url: str, timeout: num", get);
    lib.def("post", "url: str", post);
    lib.def("post", "url: str, body", post);
    lib.def("post", "url: str, body, timeout: num", post);
}

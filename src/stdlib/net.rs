//! HTTP builtins (the `net` feature and the `Net` capability).
//!
//! * `get url [options]` and `post url [body [options]]` give the response body
//!   as text, and raise an error for a status outside 200–299.
//! * `fetch url [options]` gives the whole response, `[status ok url headers
//!   body]`, whatever its status.
//!
//! The options are a map: `method`, `headers` (a map), `query` (a map added to
//! the URL), `body`, `form` (a map sent URL-encoded), `timeout` (seconds) and
//! `redirects` (`false`, or how many to follow). A body that isn't text is sent
//! as JSON.

use super::encode::query_string;
use super::{Library, checked_str, num, text};
use crate::error::{Error, ErrorKind, Result};
use crate::value::{Map, Value, to_json};
use crate::vm::Vm;

const DEFAULT_TIMEOUT: u64 = 10;
const DEFAULT_REDIRECTS: usize = 10;
const MAX_HEADERS_SIZE: usize = 64 << 10;
const OPTIONS: &str = "method, headers, query, body, form, timeout, redirects";

/// The request timeout in whole seconds (minreq's unit): the script's own,
/// clamped to 1–60 s, but no longer than the run has left (rounded up).
fn timeout(vm: &mut Vm, value: Option<&Value>) -> Result<u64> {
    // Stop before connecting if the run is already out of time or interrupted.
    vm.meter().check_now()?;
    let secs = match value {
        None | Some(Value::Null) => DEFAULT_TIMEOUT,
        Some(v @ (Value::Int(_) | Value::Float(_))) => num(v).clamp(1.0, 60.0) as u64,
        Some(other) => {
            return Err(Error::type_error(format!(
                "the `timeout` option is a number of seconds, not {}",
                other.describe()
            )));
        }
    };
    Ok(match vm.meter().remaining() {
        Some(left) => secs.min(left.as_secs() + u64::from(left.subsec_nanos() > 0)),
        None => secs,
    })
}

/// A request's settings, from the positional arguments and the options map.
#[derive(Default)]
struct Options {
    method: Option<String>,
    headers: Vec<(String, String)>,
    query: Option<Map>,
    body: Option<Value>,
    form: Option<Map>,
    timeout: Option<Value>,
    redirects: Option<Value>,
}

fn option_map(name: &str, value: &Value) -> Result<Map> {
    match value {
        Value::Map(map) => Ok(map.clone().into_map()),
        other => Err(Error::type_error(format!(
            "the `{name}` option is a map, not {}",
            other.describe()
        ))),
    }
}

fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

impl Options {
    fn parse(value: &Value) -> Result<Options> {
        let Value::Map(map) = value else {
            return Err(Error::type_error(format!(
                "request options are a map, not {}",
                value.describe()
            )));
        };
        let mut options = Options::default();
        for (key, value) in map.iter() {
            match &**key {
                "method" => {
                    let method = value.to_string().to_ascii_uppercase();
                    if !is_token(&method) {
                        return Err(Error::value_error(format!(
                            "{} is not an HTTP method",
                            value.quoted_repr()
                        )));
                    }
                    options.method = Some(method);
                }
                "headers" => {
                    for (name, value) in option_map("headers", value)? {
                        let value = value.to_string();
                        if !is_token(&name) || value.contains(['\r', '\n']) {
                            return Err(Error::value_error(format!("invalid header `{name}`")));
                        }
                        options.headers.push((name.to_string(), value));
                    }
                }
                "query" => options.query = Some(option_map("query", value)?),
                "form" => options.form = Some(option_map("form", value)?),
                "body" => options.body = Some(value.clone()),
                "timeout" => options.timeout = Some(value.clone()),
                "redirects" => options.redirects = Some(value.clone()),
                other => {
                    return Err(Error::value_error(format!(
                        "unknown request option `{other}` (the options are {OPTIONS})"
                    )));
                }
            }
        }
        Ok(options)
    }

    fn has_header(&self, name: &str) -> bool {
        self.headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case(name))
    }

    /// Builds the request for `url`, with `GET` (or `POST` when there's a
    /// body) unless a method was given.
    fn request(mut self, vm: &mut Vm, url: &str) -> Result<minreq::Request> {
        let mut url = url.to_string();
        if let Some(query) = &self.query {
            let query = query_string(query);
            if !query.is_empty() {
                url.push(if url.contains('?') { '&' } else { '?' });
                url.push_str(&query);
            }
        }
        let body = match (self.body.take(), self.form.take()) {
            (Some(_), Some(_)) => {
                return Err(Error::value_error(
                    "a request can have a `body` or a `form`, not both",
                ));
            }
            (Some(Value::Null), None) | (None, None) => None,
            (Some(Value::Str(s)), None) => Some((s.to_string(), None)),
            (Some(other), None) => Some((to_json(&other)?.to_string(), Some("application/json"))),
            (None, Some(form)) => Some((
                query_string(&form),
                Some("application/x-www-form-urlencoded"),
            )),
        };
        let method = match self.method.take() {
            Some(method) => method,
            None if body.is_some() => "POST".into(),
            None => "GET".into(),
        };
        let method = match method.as_str() {
            "GET" => minreq::Method::Get,
            "HEAD" => minreq::Method::Head,
            "POST" => minreq::Method::Post,
            "PUT" => minreq::Method::Put,
            "DELETE" => minreq::Method::Delete,
            "OPTIONS" => minreq::Method::Options,
            "PATCH" => minreq::Method::Patch,
            _ => minreq::Method::Custom(method),
        };
        let redirects = match &self.redirects {
            None | Some(Value::Null) | Some(Value::Bool(true)) => Some(DEFAULT_REDIRECTS),
            Some(Value::Bool(false)) => None,
            Some(n @ Value::Int(_)) => Some(num(n).clamp(0.0, 30.0) as usize),
            Some(other) => {
                return Err(Error::type_error(format!(
                    "the `redirects` option is a bool or a number, not {}",
                    other.describe()
                )));
            }
        };
        let mut request = minreq::Request::new(method, url)
            .with_timeout(timeout(vm, self.timeout.as_ref())?)
            .with_max_headers_size(MAX_HEADERS_SIZE)
            .with_max_status_line_length(MAX_HEADERS_SIZE);
        request = match redirects {
            Some(max) => request.with_max_redirects(max),
            None => request.with_follow_redirects(false),
        };
        if let Some((body, content_type)) = body {
            vm.meter().str_len(body.len())?;
            if let Some(content_type) = content_type.filter(|_| !self.has_header("content-type")) {
                request = request.with_header("Content-Type", content_type);
            }
            request = request.with_body(body);
        }
        Ok(request.with_headers(self.headers))
    }
}

/// A response with its body read (up to the string size limit).
struct Response {
    status: i32,
    reason: String,
    url: String,
    headers: Vec<(String, String)>,
    body: String,
}

fn send(vm: &mut Vm, request: minreq::Request) -> Result<Response> {
    let response = receive(request, vm.meter().max_str);
    // The VM only looks at the clock every few thousand fuel units, and a request
    // costs almost none, so check here: a request cut short by the run's deadline
    // reports the timeout, and a loop of requests can't outlive it.
    vm.meter().check_now()?;
    let response = response?;
    vm.meter().str_len(response.body.len())?;
    Ok(response)
}

fn receive(request: minreq::Request, max: usize) -> Result<Response> {
    let failed = |e: minreq::Error| Error::new(ErrorKind::Io, format!("request failed: {e}"));
    let response = request.send_lazy().map_err(failed)?;
    let mut headers: Vec<(String, String)> = response
        .headers
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
        .collect();
    headers.sort();
    let (status, reason, url) = (
        response.status_code,
        response.reason_phrase.clone(),
        response.url.clone(),
    );
    let mut body = Vec::new();
    for byte in response {
        let (byte, _) = byte.map_err(failed)?;
        if body.len() >= max {
            return Err(Error::limit(crate::error::Limit::StrSize));
        }
        body.push(byte);
    }
    let body = String::from_utf8(body)
        .map_err(|_| Error::new(ErrorKind::Io, "the response is not UTF-8 text"))?;
    Ok(Response {
        status,
        reason,
        url,
        headers,
        body,
    })
}

/// `get` / `post`: the body of a successful response.
fn body_of(vm: &mut Vm, url: &Value, options: Options) -> Result<Value> {
    let request = options.request(vm, text(url))?;
    let response = send(vm, request)?;
    if !(200..300).contains(&response.status) {
        return Err(Error::new(
            ErrorKind::Io,
            format!(
                "request failed with status {} {}",
                response.status, response.reason
            ),
        ));
    }
    checked_str(vm, response.body)
}

/// Options for `get`/`post` from their optional last argument: a timeout
/// (dog3's form) or an options map.
fn trailing_options(value: Option<&Value>) -> Result<Options> {
    match value {
        None => Ok(Options::default()),
        Some(options @ Value::Map(_)) => Options::parse(options),
        Some(timeout) => Ok(Options {
            timeout: Some(timeout.clone()),
            ..Options::default()
        }),
    }
}

fn get(vm: &mut Vm, args: &mut [Value]) -> Result<Value> {
    let mut options = trailing_options(args.get(1))?;
    options.method.get_or_insert_with(|| "GET".into());
    body_of(vm, &args[0], options)
}

fn post(vm: &mut Vm, args: &mut [Value]) -> Result<Value> {
    let mut options = trailing_options(args.get(2))?;
    if options.body.is_some() || options.form.is_some() {
        return Err(Error::value_error(
            "`post` takes its body as an argument, not in the options",
        ));
    }
    options.body = args.get(1).cloned();
    options.method.get_or_insert_with(|| "POST".into());
    body_of(vm, &args[0], options)
}

fn fetch(vm: &mut Vm, args: &mut [Value]) -> Result<Value> {
    let options = match args.get(1) {
        Some(options) => Options::parse(options)?,
        None => Options::default(),
    };
    let request = options.request(vm, text(&args[0]))?;
    let response = send(vm, request)?;
    let mut headers = Map::default();
    for (name, value) in response.headers {
        headers.insert(name.into(), Value::str(value));
    }
    let mut map = Map::default();
    for (key, value) in [
        ("status", Value::Int(response.status.into())),
        ("ok", Value::Bool((200..300).contains(&response.status))),
        ("url", Value::str(response.url)),
        ("headers", Value::map(headers)),
        ("body", Value::str(response.body)),
    ] {
        map.insert(key.into(), value);
    }
    Ok(Value::map(map))
}

pub(super) fn install(lib: &mut Library) {
    lib.def("get", "url: str", get);
    lib.def("get", "url: str, timeout: num", get);
    lib.def("get", "url: str, options: map", get);
    lib.def("post", "url: str", post);
    lib.def("post", "url: str, body", post);
    lib.def("post", "url: str, body, timeout: num", post);
    lib.def("post", "url: str, body, options: map", post);
    lib.def("fetch", "url: str", fetch);
    lib.def("fetch", "url: str, options: map", fetch);
}

//! Golden-file tests: every `tests/scripts/**/*.dog`, `tests/dog3/*.dog` and
//! `examples/*.dog` file is one test.
//!
//! A script's result is everything it printed followed by its final output.
//! It must equal the sibling `.out` file, or, if the script fails, the error
//! report must equal the sibling `.err` file. For dog3 fixtures (whose `.out`
//! files hold dog3's own output), a `.dog4.out` or `.dog4.err` file records an
//! intended difference; see `tests/dog3/EXPECTED_DIFFERENCES.md`.
//!
//! Header comments configure the runtime:
//!
//! * `// limits: sandbox` — run with `Limits::sandbox()`
//! * `// fuel: N` — set the fuel budget
//! * `// output: N` — set the print budget, in bytes
//! * `// deny: print` (or `eval`, `random`, `net`) — withhold a capability
//!
//! Run with `BLESS=1` to (re)write expected files from the actual results.
//! dog3's reference outputs are never overwritten.

use std::path::{Path, PathBuf};

use dog4::{Builder, Capability, FsProvider, Limits, OutputBuffer};
use libtest_mimic::{Arguments, Failed, Trial};

fn main() {
    let args = Arguments::from_args();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut trials = Vec::new();
    for dir in ["scripts", "dog3", "../examples"] {
        let mut files = Vec::new();
        collect(&root.join(dir), &mut files);
        files.sort();
        for file in files {
            let name = file
                .strip_prefix(&root)
                .unwrap_or(&file)
                .with_extension("")
                .display()
                .to_string();
            let compat = dir == "dog3";
            trials.push(Trial::test(name, move || check(&file, compat)));
        }
    }
    libtest_mimic::run(&args, trials).exit();
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "lib") {
                collect(&path, files);
            }
        } else if path.extension().is_some_and(|e| e == "dog") {
            files.push(path);
        }
    }
}

fn header(source: &str, key: &str) -> Vec<String> {
    source
        .lines()
        .take_while(|l| l.starts_with("//"))
        .filter_map(|l| l.strip_prefix(&format!("// {key}:")))
        .map(|v| v.trim().to_string())
        .collect()
}

/// Runs a script, returning its result text and whether it succeeded.
fn run(file: &Path) -> Result<(String, bool), Failed> {
    let source = std::fs::read_to_string(file)?;
    // Generous, but bounded: a runaway test must fail, not fill the disk.
    let mut limits = Limits {
        fuel: Some(200_000_000),
        timeout: Some(std::time::Duration::from_secs(20)),
        max_str_bytes: 64 << 20,
        max_collection_len: 10_000_000,
        max_output_bytes: Some(1 << 20),
        ..Limits::default()
    };
    if header(&source, "limits").iter().any(|v| v == "sandbox") {
        limits = Limits::sandbox();
    }
    if let Some(fuel) = header(&source, "fuel").first() {
        limits.fuel = Some(fuel.parse().map_err(|_| "bad fuel header")?);
    }
    if let Some(bytes) = header(&source, "output").first() {
        limits.max_output_bytes = Some(bytes.parse().map_err(|_| "bad output header")?);
    }
    let out = OutputBuffer::default();
    let mut builder = Builder::new()
        .limits(limits)
        .output(out.clone())
        .seed(42)
        .provider(FsProvider::new(file.parent().unwrap_or(Path::new("."))));
    for denied in header(&source, "deny") {
        let capability = match denied.as_str() {
            "print" => Capability::Print,
            "eval" => Capability::Eval,
            "random" => Capability::Random,
            "time" => Capability::Time,
            "net" => Capability::Net,
            other => return Err(format!("unknown capability `{other}`").into()),
        };
        builder = builder.allow(capability, false);
    }
    let mut runtime = builder.build();
    let name = file.file_stem().unwrap_or_default().to_string_lossy();
    Ok(match runtime.run(&name, &source) {
        Ok(value) => (format!("{}{value}", out.take()), true),
        Err(err) => (format!("{}{err}\n", out.take()), false),
    })
}

fn check(file: &Path, compat: bool) -> Result<(), Failed> {
    let (actual, ok) = run(file)?;
    let ext = if ok { "out" } else { "err" };
    let plain = file.with_extension(ext);
    let specific = file.with_extension(format!("dog4.{ext}"));
    let expected_path = if compat && (specific.exists() || !plain.exists()) {
        specific
    } else {
        plain
    };
    if std::env::var_os("BLESS").is_some() {
        let is_dog3_reference = compat && expected_path == file.with_extension("out");
        if !is_dog3_reference {
            if !compat {
                let stale = file.with_extension(if ok { "err" } else { "out" });
                let _ = std::fs::remove_file(stale);
            }
            std::fs::write(&expected_path, &actual)?;
            return Ok(());
        }
    }
    let expected = std::fs::read_to_string(&expected_path).map_err(|_| {
        format!(
            "missing {} (run with BLESS=1 to create it); actual result:\n{actual}",
            expected_path.display()
        )
    })?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{} differs\n--- expected\n{expected}\n--- actual\n{actual}",
            expected_path.display()
        )
        .into())
    }
}

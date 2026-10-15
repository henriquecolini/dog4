//! Executes the dog4 examples in the documentation.
//!
//! In `README.md`, `MIGRATING.md` and `docs/*.md`, every code block tagged
//! `dog` is run in a fresh runtime. If it is immediately followed by a `text`
//! block, the script's result (printed text, then its output) must match it
//! exactly; a result starting with `error:` means the script must fail with
//! that message (only the first line is compared). Untested fragments are
//! tagged `dog ignore`.

use std::path::Path;

use dog4::{Builder, MemoryProvider, OutputBuffer};
use libtest_mimic::{Arguments, Failed, Trial};

struct Example {
    file: String,
    line: usize,
    code: String,
    expected: Option<String>,
}

fn examples(file: &Path) -> Vec<Example> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() == "```dog" {
            let start = i + 1;
            let mut end = start;
            while end < lines.len() && lines[end].trim() != "```" {
                end += 1;
            }
            let code = lines[start..end].join("\n");
            let mut next = end + 1;
            while next < lines.len() && lines[next].trim().is_empty() {
                next += 1;
            }
            let mut expected = None;
            if next < lines.len() && lines[next].trim() == "```text" {
                let mut out_end = next + 1;
                while out_end < lines.len() && lines[out_end].trim() != "```" {
                    out_end += 1;
                }
                expected = Some(lines[next + 1..out_end].join("\n"));
                i = out_end;
            } else {
                i = end;
            }
            found.push(Example {
                file: file
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                line: start,
                code,
                expected,
            });
        }
        i += 1;
    }
    found
}

/// Modules the documentation imports.
fn provider() -> MemoryProvider {
    MemoryProvider::new()
        .with(
            "greet",
            "_greeting = Hello;\nfn hi (name: str) { pln $_greeting $name }\nloaded = yes",
        )
        .with(
            "shapes/area",
            "fn square (side: num) { put ($side * $side) }",
        )
}

fn check(example: &Example) -> Result<(), Failed> {
    let out = OutputBuffer::default();
    let mut rt = Builder::new()
        .output(out.clone())
        .seed(4)
        .provider(provider())
        .build();
    let result = rt.run("example", &example.code);
    let actual = match &result {
        Ok(value) => format!("{}{value}", out.take()),
        Err(e) => format!("{}{e}", out.take()),
    };
    let Some(expected) = &example.expected else {
        return match result {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("example failed:\n{e}").into()),
        };
    };
    let matches = if expected.starts_with("error:") || expected.contains("\nerror:") {
        let first_lines = |s: &str| {
            let mut lines: Vec<String> = Vec::new();
            for line in s.lines() {
                lines.push(line.to_string());
                if line.starts_with("error:") {
                    break;
                }
            }
            lines
        };
        result.is_err() && first_lines(&actual) == first_lines(expected)
    } else {
        actual.trim_end_matches('\n') == expected.trim_end_matches('\n')
    };
    if matches {
        Ok(())
    } else {
        Err(format!(
            "{}:{} doesn't produce what the docs say\n--- documented\n{expected}\n--- actual\n{actual}",
            example.file, example.line
        )
        .into())
    }
}

fn main() {
    let args = Arguments::from_args();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![root.join("README.md"), root.join("MIGRATING.md")];
    if let Ok(entries) = std::fs::read_dir(root.join("docs")) {
        let mut docs: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .collect();
        docs.sort();
        files.extend(docs);
    }
    let mut trials = Vec::new();
    for file in files {
        for example in examples(&file) {
            let name = format!("{}:{}", example.file, example.line);
            trials.push(Trial::test(name, move || check(&example)));
        }
    }
    libtest_mimic::run(&args, trials).exit();
}

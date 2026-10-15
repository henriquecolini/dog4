//! The `dog4` command-line interpreter.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use dog4::{Builder, FsProvider, Value};

/// Runs dog4 scripts.
#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// Script to run. Without one (and without `-e`), starts a REPL.
    file: Option<PathBuf>,
    /// Run this code instead of a file.
    #[arg(short = 'e', long = "eval", value_name = "CODE")]
    code: Option<String>,
    /// Print the compiled bytecode instead of running.
    #[arg(long)]
    disasm: bool,
    /// Run with the sandbox limits used for untrusted code.
    #[arg(long)]
    sandbox: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let mut builder = if args.sandbox {
        Builder::sandboxed()
    } else {
        Builder::new()
    };
    let (name, source) = match (&args.code, &args.file) {
        (Some(code), _) => {
            builder = builder.provider(FsProvider::new("."));
            ("main".to_string(), code.clone())
        }
        (None, Some(file)) => {
            let source = match std::fs::read_to_string(file) {
                Ok(source) => source,
                Err(e) => {
                    eprintln!("error: can't read {}: {e}", file.display());
                    return ExitCode::FAILURE;
                }
            };
            let root = file.parent().map(PathBuf::from).unwrap_or_default();
            builder = builder.provider(FsProvider::new(root));
            let name = file
                .file_stem()
                .map_or("main".into(), |s| s.to_string_lossy().into_owned());
            (name, source)
        }
        (None, None) => {
            let runtime = builder.provider(FsProvider::new(".")).build();
            return repl(runtime);
        }
    };
    let mut runtime = builder.build();
    if args.disasm {
        return match runtime.disassemble(&name, &source) {
            Ok(listing) => {
                print!("{listing}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }
    match runtime.run(&name, &source) {
        Ok(output) => {
            print!("{output}");
            let _ = std::io::stdout().flush();
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = std::io::stdout().flush();
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn repl(mut runtime: dog4::Runtime) -> ExitCode {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        print!("dog4> ");
        let _ = std::io::stdout().flush();
        let Some(Ok(line)) = lines.next() else {
            println!();
            return ExitCode::SUCCESS;
        };
        if line.trim().is_empty() {
            continue;
        }
        match runtime.eval(&line) {
            Ok(Value::Null) => {}
            Ok(value) => {
                let text = value.to_string();
                print!("{text}");
                if !text.ends_with('\n') {
                    println!();
                }
            }
            Err(e) => eprintln!("{e}"),
        }
    }
}

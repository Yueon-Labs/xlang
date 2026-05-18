use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::driver::{build_exe, parse_file, write_c};
use crate::error::{XError, XResult};

pub fn run_cli() -> XResult<()> {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args[0] == "help" || args[0] == "--help" || args[0] == "-h" {
        print_help();
        return Ok(());
    }

    let cmd = args.remove(0);
    match cmd.as_str() {
        "check" => {
            if args.is_empty() {
                return Err(XError::Parse("usage: xlangc check <files...>".into()));
            }
            let mut ok = true;
            for file in args {
                match parse_file(Path::new(&file)) {
                    Ok(_) => println!("[ok] {file}"),
                    Err(err) => {
                        ok = false;
                        eprintln!("[error] {file}: {err}");
                    }
                }
            }
            if ok {
                Ok(())
            } else {
                Err(XError::Parse("one or more files failed".into()))
            }
        }
        "ast" => {
            let source = one_source_arg(&args, "xlangc ast <file>")?;
            let program = parse_file(Path::new(source))?;
            println!("{}", serde_json::to_string_pretty(&program)?);
            Ok(())
        }
        "c" => {
            let (source, output) = parse_source_o(&args, "xlangc c <file> [-o output.c]")?;
            let path = write_c(Path::new(&source), output.map(PathBuf::from))?;
            println!("{}", path.display());
            Ok(())
        }
        "build" => {
            let (source, output) = parse_source_o(&args, "xlangc build <file> [-o output]")?;
            let path = build_exe(Path::new(&source), output.map(PathBuf::from))?;
            println!("{}", path.display());
            Ok(())
        }
        "run" => {
            let (source, output) = parse_source_o(&args, "xlangc run <file> [-o output]")?;
            let exe = build_exe(Path::new(&source), output.map(PathBuf::from))?;
            let out = Command::new(fs::canonicalize(exe)?).output()?;
            print!("{}", String::from_utf8_lossy(&out.stdout));
            eprint!("{}", String::from_utf8_lossy(&out.stderr));
            println!(
                "program exited with code {}",
                out.status.code().unwrap_or(-1)
            );
            Ok(())
        }
        other => Err(XError::Parse(format!("unknown command {other:?}"))),
    }
}

fn print_help() {
    println!(
        "xlangc - minimal X Language compiler prototype\n\n\
         Commands:\n\
           xlangc check <files...>        Parse files\n\
           xlangc ast <file>              Print JSON AST\n\
           xlangc c <file> [-o out.c]     Generate C for supported subset\n\
           xlangc build <file> [-o out]   Build native executable\n\
           xlangc run <file> [-o out]     Build and run"
    );
}

fn one_source_arg<'a>(args: &'a [String], usage: &str) -> XResult<&'a str> {
    if args.len() != 1 {
        return Err(XError::Parse(format!("usage: {usage}")));
    }
    Ok(&args[0])
}

fn parse_source_o(args: &[String], usage: &str) -> XResult<(String, Option<String>)> {
    if args.is_empty() {
        return Err(XError::Parse(format!("usage: {usage}")));
    }
    let source = args[0].clone();
    let mut output = None;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "-o" || args[i] == "--output" {
            if i + 1 >= args.len() {
                return Err(XError::Parse(format!("usage: {usage}")));
            }
            output = Some(args[i + 1].clone());
            i += 2;
        } else {
            return Err(XError::Parse(format!("usage: {usage}")));
        }
    }
    Ok((source, output))
}

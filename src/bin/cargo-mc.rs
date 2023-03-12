/*
 * @Author: Shuwen Chen 
 * @Date: 2023-03-12 16:43:53 
 * @Last Modified by: Shuwen Chen
 * @Last Modified time: 2023-03-12 17:02:08
 */
use serde_json;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

const CARGO_MEMORY_CHECK_HELP: &str = r#" rust memory check for UAF(use after free), DP(dangling pointer), DF(double free)

Usage:
    cargo mc
"#;

fn show_help() {
    println!("{}", CARGO_MEMORY_CHECK_HELP);
}

fn show_version() {
    println!("mc {}", env!("CARGO_PKG_VERSION"));
}
fn show_error(msg: String) -> ! {
    eprintln!("fatal error: {}", msg);
    std::process::exit(1)
}

// Determines whether a flag `name` is present before `--`.
// For example, has_arg_flag("-v")
fn has_arg_flag(name: &str) -> bool {
    let mut args = std::env::args().take_while(|val| val != "--");
    args.any(|val| val == name)
}

// Gets the value of a `name`.
// For example, get_arg_flag_value("--manifest-path")
// Supports two styles: `--name value` or `--name=value`
fn get_arg_flag_value(name: &str) -> Option<String> {
    // Stop searching at `--`.
    let mut args = std::env::args().take_while(|val| val != "--");
    loop {
        let arg = match args.next() {
            Some(arg) => arg,
            None => return None,
        };
        if !arg.starts_with(name) {
            continue;
        }
        // Strip leading `name`.
        let suffix = &arg[name.len()..];
        if suffix.is_empty() {
            // This argument is exactly `name`; the next one is the value.
            return args.next();
        } else if suffix.starts_with('=') {
            // This argument is `name=value`; get the value.
            // Strip leading `=`.
            return Some(suffix[1..].to_owned());
        }
    }
}

// Get the top level crate that we need to analyze
fn current_crate() -> cargo_metadata::Package {
    // We need to get the manifest, and then the metadata, to enumerate targets.

    // Path to the `Cargo.toml` file
    let manifest_path =
        get_arg_flag_value("--manifest-path").map(|m| Path::new(&m).canonicalize().unwrap());

    let mut cmd = cargo_metadata::MetadataCommand::new();
    if let Some(ref manifest_path) = manifest_path {
        cmd.manifest_path(manifest_path);
    }
    let mut metadata = if let Ok(metadata) = cmd.exec() {
        metadata
    } else {
        show_error("Could not obtain Cargo metadata; likely an ill-formed manifest".to_string());
    };

    let current_dir = std::env::current_dir();

    let package_index = metadata
        .packages
        .iter()
        .position(|package| {
            let package_manifest_path = Path::new(&package.manifest_path);
            if let Some(ref manifest_path) = manifest_path {
                package_manifest_path == manifest_path
            } else {
                let current_dir = current_dir
                    .as_ref()
                    .expect("could not read current directory");
                let package_manifest_directory = package_manifest_path
                    .parent()
                    .expect("could not find parent directory of package manifest");
                package_manifest_directory == current_dir
            }
        })
        .unwrap_or_else(|| {
            show_error(
                "This seems to be a workspace, which is not supported by cargo-mc".to_string(),
            )
        });
    let package = metadata.packages.remove(package_index);

    package
}

fn mc() -> Command {
    let mut path = std::env::current_exe().expect("current executable path invalid");
    path.set_file_name("mc");
    Command::new(path)
}

fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")))
}

fn main() {
    // Check for version and help flags even when invoked as `cargo-mc`.
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        show_help();
        return;
    }
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        show_version();
        return;
    }
    
    if let Some("mc") = std::env::args().nth(1).as_ref().map(AsRef::as_ref) {
        // This arm is for when `cargo mc` is called. We call `cargo rustc` for each applicable target,
        // but with the `RUSTC` env var set to the `cargo-mc` binary so that we come back in the other branch,
        // and dispatch the invocations to `rustc` and `mc`, respectively.
        in_cargo_mc();
    } else if let Some("rustc") = std::env::args().nth(1).as_ref().map(AsRef::as_ref) {
        // This arm is executed when `cargo-mc` runs `cargo rustc` with the `RUSTC_WRAPPER` env var set to itself:
        // dependencies get dispatched to `rustc`, the final library/binary to `mc`.
        inside_cargo_rustc();
    } else {
        show_error(
            "`cargo-mc` must be called with either `mc` or `rustc` as first argument.".to_string(),
        )
    }
}

// This will construct command line like:
// `cargo rustc --bin some_crate_name -v -- cargo-mc-marker-begin --top_crate_name some_top_crate_name --domain interval -v cargo-mc-marker-end`
// And set the following environment variables:
// `RUSTC_WRAPPER` is set to `cargo-mc` itself so the execution will come back to the second branch as described above
// `MEMORY_CHECK_ARGS` is set to the user-provided arguments for `mc`
// `MEMORY_CHECK_TOP_CRATE_NAME` is set to the name of the crate being analyzed
// `MEMORY_CHECK_VERBOSE` is set if `-v` is provided
fn in_cargo_mc() {
    let verbose = has_arg_flag("-v");

    let current_crate = current_crate();

    // Now run the command.
    for target in current_crate.targets.into_iter() {
        let mut args = std::env::args().skip(2);
        let kind = target
            .kind
            .get(0)
            .expect("badly formatted cargo metadata: target::kind is an empty array");

        // Now we run `cargo rustc $FLAGS $ARGS`, giving the user the
        // chance to add additional arguments. `FLAGS` is set to identify
        // this target.  The user gets to control what gets actually passed to mc.
        let mut cmd = cargo();
        cmd.arg("check"); // using `check` may speed up the analysis than using `rustc`
        match kind.as_str() {
            "bin" => {
                cmd.arg("--bin").arg(target.name);
            }
            "lib" => {
                cmd.arg("--lib");
            }
            _ => continue,
        }

        // Add cargo args until first `--`.
        while let Some(arg) = args.next() {
            if arg == "--" {
                break;
            }
            cmd.arg(arg);
        }

        // Serialize the remaining args into a special environemt variable.
        // This will be read by `inside_cargo_rustc` when we go to invoke
        // our actual target crate.
        // Since we're using "cargo check", we have no other way of passing
        // these arguments.
        // We also add `MEMORY_CHECK_TOP_CRATE_NAME` to specify the top-level
        // crate name that we want to analyze, by doing this we can dispatch
        // dependencies to the real `rustc` and top-level crate to `mc`
        let args_vec: Vec<String> = args.collect();
        cmd.env(
            "MEMORY_CHECK_ARGS",
            serde_json::to_string(&args_vec).expect("failed to serialize args"),
        );
        cmd.env("MEMORY_CHECK_TOP_CRATE_NAME", current_crate.name.clone());

        // Replace the rustc executable through RUSTC_WRAPPER environment variable
        let path = std::env::current_exe().expect("current executable path invalid");
        cmd.env("RUSTC_WRAPPER", path);

        if verbose {
            cmd.env("MEMORY_CHECK_VERBOSE", ""); // this makes `inside_cargo_rustc` verbose.
            eprintln!("+ {:?}", cmd);
        }

        // Execute cmd
        let exit_status = cmd
            .spawn()
            .expect("could not run cargo")
            .wait()
            .expect("failed to wait for cargo?");

        if !exit_status.success() {
            std::process::exit(exit_status.code().unwrap_or(-1))
        }
    }
}

// This will construct command line like:
// `mc --crate-name some_crate_name --edition=2018 src/lib.rs --crate-type lib --domain interval`
// And sets the environment variable `MEMORY_CHECK_BE_RUSTC`
// if `mc` is going to analyze crates that are dependencies
fn inside_cargo_rustc() {
    let mut cmd = mc();
    cmd.args(std::env::args().skip(2)); // skip `cargo-mc rustc`

    // Add sysroot
    let sysroot = compile_time_sysroot().expect("Cannot find sysroot");
    cmd.arg("--sysroot");
    cmd.arg(sysroot);

    let top_crate_name =
        std::env::var("MEMORY_CHECK_TOP_CRATE_NAME").expect("missing MEMORY_CHECK_TOP_CRATE_NAME");
    let top_crate_name = top_crate_name.replace("-", "_"); // Cargo seems to rename hyphens to underscores

    if get_arg_flag_value("--crate-name").as_deref() == Some(&top_crate_name) {
        // If we are analyzing the crate that we want to analyze, add args for `mc`
        let magic = std::env::var("MEMORY_CHECK_ARGS").expect("missing MEMORY_CHECK_ARGS");
        let mc_args: Vec<String> =
            serde_json::from_str(&magic).expect("failed to deserialize MEMORY_CHECK_ARGS");
        cmd.args(mc_args);
    } else {
        // If we are analyzing dependencies, set this environment variable so
        // that `mc` will behave just like the real `rustc` and do the
        // compilation instead of analysis
        cmd.env("MEMORY_CHECK_BE_RUSTC", "1");
    }

    let verbose = std::env::var_os("MEMORY_CHECK_VERBOSE").is_some();
    if verbose {
        eprintln!("+ {:?}", cmd);
    }

    match cmd.status() {
        Ok(exit) => {
            if !exit.success() {
                std::process::exit(exit.code().unwrap_or(42));
            }
        }
        Err(ref e) => panic!("error during mc run: {:?}", e),
    }
}

fn compile_time_sysroot() -> Option<String> {
    if option_env!("RUST_STAGE").is_some() {
        return None;
    }
    let home = option_env!("RUSTUP_HOME").or(option_env!("MULTIRUST_HOME"));
    let toolchain = option_env!("RUSTUP_TOOLCHAIN").or(option_env!("MULTIRUST_TOOLCHAIN"));
    Some(match (home, toolchain) {
        (Some(home), Some(toolchain)) => format!("{}/toolchains/{}", home, toolchain),
        _ => option_env!("RUST_SYSROOT")
            .expect(
                "To build without rustup, set the `RUST_SYSROOT` env var at build time",
            )
            .to_owned(),
    })
}
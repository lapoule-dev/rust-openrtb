//! `cargo xtask codegen [--check]`
//!
//! 1. Compiles the official IAB `openrtb.proto` to a descriptor set (protoc).
//! 2. Runs buffa on it → `crates/openrtb-model/src/generated/buffa/`.
//! 3. Walks the same descriptors and emits the OpenRTB JSON codec and the
//!    spec-default getters → `crates/openrtb-model/src/generated/openrtb.rs`.
//!
//! Generated sources are committed so users need neither protoc nor this tool.
//! `--check` regenerates into a temp dir and fails if anything differs.

mod json;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use buffa::Message;
use buffa_codegen::CodeGenConfig;
use buffa_codegen::context::CodeGenContext;
use buffa_descriptor::generated::descriptor::FileDescriptorSet;

const PROTO_ROOT: &str = "reference/openrtb2.x/proto/src/main";
const PROTO_FILE: &str = "com/iabtechlab/openrtb/v2/openrtb.proto";
const MODEL_GENERATED: &str = "crates/openrtb-model/src/generated";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let check = args.iter().any(|a| a == "--check");
    match args.first().map(String::as_str) {
        Some("codegen") => match run(check) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage: cargo xtask codegen [--check]");
            ExitCode::FAILURE
        }
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn run(check: bool) -> Result<(), Box<dyn std::error::Error>> {
    let root = workspace_root();
    let final_dir = root.join(MODEL_GENERATED);
    let tmp = root.join("target/xtask-codegen");
    let out_dir = if check {
        tmp.join("generated")
    } else {
        final_dir.clone()
    };
    if out_dir.exists() {
        std::fs::remove_dir_all(&out_dir)?;
    }
    std::fs::create_dir_all(out_dir.join("buffa"))?;

    let descriptor_path = protoc_descriptor_set(&root, &tmp)?;
    let descriptor_bytes = std::fs::read(&descriptor_path)?;

    buffa_build::Config::new()
        .files(&[PROTO_FILE])
        .includes(&[root.join(PROTO_ROOT)])
        .descriptor_set(&descriptor_path)
        .generate_views(true)
        .lazy_views(true)
        .generate_json(false)
        .out_dir(out_dir.join("buffa"))
        .include_file("mod.rs")
        .compile()?;

    let set = FileDescriptorSet::decode_from_slice(&descriptor_bytes)?;
    let files_to_generate = vec![PROTO_FILE.to_owned()];
    let config = CodeGenConfig::default();
    let ctx = CodeGenContext::for_generate(&set.file, &files_to_generate, &config);
    let code = json::generate(&ctx, &set.file, &files_to_generate);
    let path = out_dir.join("openrtb.rs");
    // Empty template slots leave whitespace-only lines that rustfmt rejects.
    let code: String = code
        .lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(&path, code)?;
    rustfmt(&path)?;

    if check {
        let diff = Command::new("diff")
            .arg("-r")
            .arg(&final_dir)
            .arg(&out_dir)
            .status()?;
        if !diff.success() {
            return Err("generated code is stale: run `cargo xtask codegen`".into());
        }
        println!("generated code is up to date");
    } else {
        println!("wrote {}", final_dir.display());
    }
    Ok(())
}

fn protoc_descriptor_set(root: &Path, tmp: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(tmp)?;
    let out = tmp.join("openrtb.binpb");
    let protoc = std::env::var("PROTOC").unwrap_or_else(|_| "protoc".into());
    let status = Command::new(protoc)
        .arg("--include_imports")
        .arg("--include_source_info")
        .arg(format!("-I{}", root.join(PROTO_ROOT).display()))
        .arg(format!("--descriptor_set_out={}", out.display()))
        .arg(PROTO_FILE)
        .current_dir(root.join(PROTO_ROOT))
        .status()?;
    if !status.success() {
        return Err("protoc failed".into());
    }
    Ok(out)
}

fn rustfmt(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("rustfmt")
        .arg("--edition=2024")
        .arg(path)
        .status()?;
    if !status.success() {
        return Err(format!("rustfmt failed on {}", path.display()).into());
    }
    Ok(())
}

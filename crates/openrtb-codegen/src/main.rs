//! `cargo xtask codegen [--check]`
//!
//! 1. AdCOM 1.0 spec (markdown) → `proto/com/iabtechlab/adcom/v1/{adcom,enums}.proto`.
//! 2. For each target (AdCOM, then the official IAB OpenRTB 2.x proto):
//!    protoc → descriptor set → buffa (`<crate>/src/generated/buffa/`) and our
//!    generator (`<crate>/src/generated/<name>.rs`: OpenRTB JSON codec and
//!    spec-default getters) from the same descriptors.
//!
//! Generated sources are committed so users need neither protoc nor this tool.
//! `--check` regenerates into a temp dir and fails if anything differs.

mod adcom;
mod json;
mod spec;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use buffa::Message;
use buffa_codegen::CodeGenConfig;
use buffa_codegen::context::CodeGenContext;
use buffa_descriptor::generated::descriptor::FileDescriptorSet;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const ADCOM_SPEC: &str = "reference/AdCOM/AdCOM v1.0 FINAL.md";
const OUR_PROTOS: &str = "proto";

struct Target {
    name: &'static str,
    proto_root: &'static str,
    files: &'static [&'static str],
    crate_dir: &'static str,
}

const TARGETS: &[Target] = &[
    Target {
        name: "adcom",
        proto_root: OUR_PROTOS,
        files: &[
            "com/iabtechlab/adcom/v1/enums.proto",
            "com/iabtechlab/adcom/v1/adcom.proto",
        ],
        crate_dir: "crates/adcom",
    },
    Target {
        name: "openrtb",
        proto_root: "reference/openrtb2.x/proto/src/main",
        files: &["com/iabtechlab/openrtb/v2/openrtb.proto"],
        crate_dir: "crates/openrtb-model",
    },
];

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

fn run(check: bool) -> Result<()> {
    let root = workspace_root();
    let tmp = root.join("target/xtask-codegen");
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir_all(&tmp)?;
    let mut stale = Vec::new();

    // 1. AdCOM spec → protos.
    let proto_dir = if check {
        tmp.join("proto")
    } else {
        root.join(OUR_PROTOS)
    };
    let md = std::fs::read_to_string(root.join(ADCOM_SPEC))?;
    let spec = spec::parse(&md);
    let generated = adcom::generate(&spec);
    let adcom_dir = proto_dir.join("com/iabtechlab/adcom/v1");
    std::fs::create_dir_all(&adcom_dir)?;
    std::fs::write(adcom_dir.join("enums.proto"), &generated.enums_proto)?;
    let adcom_proto = adcom::resolve_enum_defaults(&generated.adcom_proto, &generated.enums_proto);
    std::fs::write(adcom_dir.join("adcom.proto"), adcom_proto)?;
    if check && !same_tree(&root.join(OUR_PROTOS), &proto_dir)? {
        stale.push(OUR_PROTOS);
    }

    // 2. Protos → Rust, per target.
    for target in TARGETS {
        let final_dir = root.join(target.crate_dir).join("src/generated");
        let out_dir = if check {
            tmp.join(target.name)
        } else {
            final_dir.clone()
        };
        // In check mode, compile the freshly generated protos.
        let proto_root = if check && target.proto_root == OUR_PROTOS {
            proto_dir.clone()
        } else {
            root.join(target.proto_root)
        };
        generate_target(target, &proto_root, &out_dir, &tmp)?;
        if check && !same_tree(&final_dir, &out_dir)? {
            stale.push(target.crate_dir);
        }
    }

    if check {
        if !stale.is_empty() {
            return Err(format!(
                "generated code is stale in {}: run `cargo xtask codegen`",
                stale.join(", ")
            )
            .into());
        }
        println!("generated code is up to date");
    } else {
        println!("generated code written");
    }
    Ok(())
}

fn generate_target(target: &Target, proto_root: &Path, out_dir: &Path, tmp: &Path) -> Result<()> {
    if out_dir.exists() {
        std::fs::remove_dir_all(out_dir)?;
    }
    std::fs::create_dir_all(out_dir.join("buffa"))?;
    let descriptor_path = tmp.join(format!("{}.binpb", target.name));
    protoc(proto_root, target.files, &descriptor_path)?;

    buffa_build::Config::new()
        .files(target.files)
        .includes(&[proto_root])
        .descriptor_set(&descriptor_path)
        .generate_views(true)
        .lazy_views(true)
        .generate_json(false)
        .out_dir(out_dir.join("buffa"))
        .include_file("mod.rs")
        .compile()?;
    // buffa's code is not ours to lint: new clippy releases must not break the build.
    let include = out_dir.join("buffa/mod.rs");
    let code = std::fs::read_to_string(&include)?.replacen(
        "pub mod com {",
        "#[allow(clippy::all)]\npub mod com {",
        1,
    );
    std::fs::write(&include, code)?;

    let set = FileDescriptorSet::decode_from_slice(&std::fs::read(&descriptor_path)?)?;
    let files_to_generate: Vec<String> = target.files.iter().map(|f| (*f).to_owned()).collect();
    let config = CodeGenConfig::default();
    let ctx = CodeGenContext::for_generate(&set.file, &files_to_generate, &config);
    let code = json::generate(&ctx, &set.file, &files_to_generate);
    let path = out_dir.join(format!("{}.rs", target.name));
    // Empty template slots leave whitespace-only lines that rustfmt rejects.
    let code: String = code
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(&path, code)?;
    rustfmt(&path)
}

fn protoc(proto_root: &Path, files: &[&str], out: &Path) -> Result<()> {
    let protoc = std::env::var("PROTOC").unwrap_or_else(|_| "protoc".into());
    let status = Command::new(protoc)
        .arg("--include_imports")
        .arg("--include_source_info")
        .arg(format!("-I{}", proto_root.display()))
        .arg(format!("--descriptor_set_out={}", out.display()))
        .args(files)
        .current_dir(proto_root)
        .status()?;
    if !status.success() {
        return Err("protoc failed".into());
    }
    Ok(())
}

fn same_tree(a: &Path, b: &Path) -> Result<bool> {
    Ok(Command::new("diff")
        .arg("-r")
        .arg(a)
        .arg(b)
        .status()?
        .success())
}

fn rustfmt(path: &Path) -> Result<()> {
    let status = Command::new("rustfmt")
        .arg("--edition=2024")
        .arg(path)
        .status()?;
    if !status.success() {
        return Err(format!("rustfmt failed on {}", path.display()).into());
    }
    Ok(())
}

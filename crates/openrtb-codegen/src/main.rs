//! `cargo xtask codegen [--check]`
//!
//! 1. Specs (markdown) → our protos in `proto/`: AdCOM 1.0 objects and lists,
//!    OpenRTB 3.0 lists (No-Bid and Loss Reason Codes, used by 2.6).
//! 2. For each target crate: protoc → descriptor set → buffa
//!    (`<crate>/src/generated/buffa/`) and our generator
//!    (`<crate>/src/generated/<name>.rs`: OpenRTB JSON codec, spec-default
//!    getters, typed accessors for list-valued integers) from the same descriptors.
//!
//! Generated sources are committed so users need neither protoc nor this tool.
//! `--check` regenerates into a temp dir and fails if anything differs.

mod adcom;
mod json;
mod spec;
mod typed;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use buffa::Message;
use buffa_codegen::CodeGenConfig;
use buffa_codegen::context::CodeGenContext;
use buffa_descriptor::generated::descriptor::FileDescriptorSet;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const ADCOM_SPEC: &str = "reference/AdCOM/AdCOM v1.0 FINAL.md";
const OPENRTB3_SPEC: &str = "reference/openrtb/OpenRTB v3.0 FINAL.md";
const OPENRTB26_SPEC: &str = "reference/openrtb2.x/2.6.md";
const OUR_PROTOS: &str = "proto";

struct Target {
    name: &'static str,
    /// `(include root, proto file relative to it)`.
    files: &'static [(&'static str, &'static str)],
    crate_dir: &'static str,
    /// Generate typed accessors for the OpenRTB 2.x integer fields holding list values.
    typed_lists: bool,
}

const TARGETS: &[Target] = &[
    Target {
        name: "adcom",
        files: &[
            (OUR_PROTOS, "com/iabtechlab/adcom/v1/enums.proto"),
            (OUR_PROTOS, "com/iabtechlab/adcom/v1/adcom.proto"),
        ],
        crate_dir: "crates/adcom",
        typed_lists: false,
    },
    Target {
        name: "openrtb",
        files: &[
            (
                "reference/openrtb2.x/proto/src/main",
                "com/iabtechlab/openrtb/v2/openrtb.proto",
            ),
            (OUR_PROTOS, "com/iabtechlab/openrtb/v3/enums.proto"),
        ],
        crate_dir: "crates/openrtb-model",
        typed_lists: true,
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

    // 1. Specs → protos.
    let proto_dir = if check {
        tmp.join("proto")
    } else {
        root.join(OUR_PROTOS)
    };
    let adcom_spec = spec::parse(&std::fs::read_to_string(root.join(ADCOM_SPEC))?);
    let generated = adcom::generate(&adcom_spec);
    let adcom_dir = proto_dir.join("com/iabtechlab/adcom/v1");
    std::fs::create_dir_all(&adcom_dir)?;
    std::fs::write(adcom_dir.join("enums.proto"), &generated.enums_proto)?;
    let adcom_proto = adcom::resolve_enum_defaults(&generated.adcom_proto, &generated.enums_proto);
    std::fs::write(adcom_dir.join("adcom.proto"), adcom_proto)?;

    let openrtb3_spec = spec::parse(&std::fs::read_to_string(root.join(OPENRTB3_SPEC))?);
    let v3_dir = proto_dir.join("com/iabtechlab/openrtb/v3");
    std::fs::create_dir_all(&v3_dir)?;
    std::fs::write(
        v3_dir.join("enums.proto"),
        adcom::openrtb3_enums_proto(&openrtb3_spec),
    )?;
    if check && !same_tree(&root.join(OUR_PROTOS), &proto_dir)? {
        stale.push(OUR_PROTOS);
    }

    let lists = typed::known_lists(&adcom_spec.lists, &openrtb3_spec.lists);
    let spec26 = std::fs::read_to_string(root.join(OPENRTB26_SPEC))?;

    // 2. Protos → Rust, per target.
    for target in TARGETS {
        let final_dir = root.join(target.crate_dir).join("src/generated");
        let out_dir = if check {
            tmp.join(target.name)
        } else {
            final_dir.clone()
        };
        // In check mode, compile the freshly generated protos.
        let resolve = |r: &str| {
            if check && r == OUR_PROTOS {
                proto_dir.clone()
            } else {
                root.join(r)
            }
        };
        let inputs = Inputs {
            resolve: &resolve,
            lists: &lists,
            spec26: &spec26,
        };
        generate_target(target, &inputs, &out_dir, &tmp)?;
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

struct Inputs<'a> {
    resolve: &'a dyn Fn(&str) -> PathBuf,
    lists: &'a [typed::KnownList],
    spec26: &'a str,
}

fn generate_target(target: &Target, inputs: &Inputs, out_dir: &Path, tmp: &Path) -> Result<()> {
    if out_dir.exists() {
        std::fs::remove_dir_all(out_dir)?;
    }
    std::fs::create_dir_all(out_dir.join("buffa"))?;
    let mut includes: Vec<PathBuf> = Vec::new();
    for (r, _) in target.files {
        let r = (inputs.resolve)(r);
        if !includes.contains(&r) {
            includes.push(r);
        }
    }
    let files: Vec<PathBuf> = target
        .files
        .iter()
        .map(|(r, f)| (inputs.resolve)(r).join(f))
        .collect();
    let descriptor_path = tmp.join(format!("{}.binpb", target.name));
    protoc(&includes, &files, &descriptor_path)?;

    let relative: Vec<&str> = target.files.iter().map(|(_, f)| *f).collect();
    buffa_build::Config::new()
        .files(&relative)
        .includes(&includes)
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
    let files_to_generate: Vec<String> =
        target.files.iter().map(|(_, f)| (*f).to_owned()).collect();
    let config = CodeGenConfig::default();
    let ctx = CodeGenContext::for_generate(&set.file, &files_to_generate, &config);
    let field_enums = if target.typed_lists {
        typed::field_enums(
            &ctx,
            &set.file,
            &files_to_generate,
            inputs.spec26,
            inputs.lists,
        )
    } else {
        Default::default()
    };
    let code = json::generate(&ctx, &set.file, &files_to_generate, &field_enums);
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

fn protoc(includes: &[PathBuf], files: &[PathBuf], out: &Path) -> Result<()> {
    let protoc = std::env::var("PROTOC").unwrap_or_else(|_| "protoc".into());
    let status = Command::new(protoc)
        .arg("--include_imports")
        .arg("--include_source_info")
        .args(includes.iter().map(|i| format!("-I{}", i.display())))
        .arg(format!("--descriptor_set_out={}", out.display()))
        .args(files)
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

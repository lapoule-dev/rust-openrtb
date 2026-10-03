const PROTO_DIR: &str = "../../reference/openrtb2.x/proto/src/main";

fn main() {
    buffa_build::Config::new()
        .files(&[format!("{PROTO_DIR}/com/iabtechlab/openrtb/v2/openrtb.proto")])
        .includes(&[PROTO_DIR])
        .generate_views(true)
        .lazy_views(true)
        .generate_json(false)
        .include_file("_include.rs")
        .compile()
        .expect("openrtb.proto compilation failed");
}

//! Which OpenRTB 2.x integer fields hold values of an enumerated list.
//!
//! The official proto types them `int32` (enums were left open-ended), so the
//! mapping is recovered from two sources, merged:
//! - field comments of the proto: "Refer to enum com.iabtechlab.adcom.v1.enums.DeviceType";
//! - attribute tables of the 2.6 spec: "Refer to [List: Device Types](…AdCOM…)".
//!
//! The result maps a field (`com.iabtechlab.openrtb.v2.BidRequest.Device.devicetype`)
//! to the Rust path of its enum (`::adcom::enums::DeviceType`).

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use buffa_codegen::context::CodeGenContext;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
use buffa_descriptor::generated::descriptor::{DescriptorProto, FileDescriptorProto};
use regex::Regex;

use crate::spec::List;

/// Enum names used by the proto comments (IABTechLab/adcom-proto) that we spell differently.
const PROTO_ENUM_ALIASES: &[(&str, &str)] = &[
    ("APIFramework", "ApiFramework"),
    ("Creative.Attribute", "CreativeAttribute"),
    ("Creative.AudioVideoType", "CreativeSubtypeAudioVideo"),
    ("Creative.DisplayType", "CreativeSubtypeDisplay"),
    ("DOOHVenueType", "DoohVenueType"),
    ("DOOHVenueTaxonomy", "DoohVenueTaxonomy"),
    (
        "DOOHMultiplierMeasurementSourceType",
        "DoohMultiplierMeasurementSourceType",
    ),
];

/// Fields whose list only names special values (`startdelay` > 0 is seconds).
const NOT_ENUM_FIELDS: &[&str] = &["startdelay"];

static PROTO_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"com\.iabtechlab\.adcom\.v1\.enums\.([A-Za-z]+(?:\.[A-Z][A-Za-z]+)?)").unwrap()
});
static MD_OBJECT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^###\s+[\d.]+\s*-?\s*Object:\s*(\w+)").unwrap());
static MD_ROW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\|\s*`(\w+)`\s*\|([^|]*)\|(.*)$").unwrap());
static MD_LIST_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Refer to \[List:\s*([^\]]+)\]\(([^)]*)\)").unwrap());

/// An enumerated list the generated crates know: its title, anchor and Rust path.
pub struct KnownList {
    pub title: String,
    pub anchor: String,
    pub rust_path: String,
}

pub fn known_lists(adcom: &[List], openrtb3: &[List]) -> Vec<KnownList> {
    let mut out = Vec::new();
    for (lists, prefix) in [
        (adcom, "::adcom::enums"),
        (openrtb3, "crate::com::iabtechlab::openrtb::v3"),
    ] {
        for l in lists {
            let name = crate::adcom::enum_name(&l.anchor)
                .unwrap_or_else(|| panic!("no enum for {}", l.anchor));
            out.push(KnownList {
                title: l.title.clone(),
                anchor: l.anchor.clone(),
                rust_path: format!("{prefix}::{name}"),
            });
        }
    }
    out
}

pub fn field_enums(
    ctx: &CodeGenContext,
    files: &[FileDescriptorProto],
    to_generate: &[String],
    spec_md: &str,
    lists: &[KnownList],
) -> HashMap<String, String> {
    // Every int32 field, by "Message.field" (simple message name) and by fqn.
    let mut int_fields: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut all = Vec::new();
    for file in files
        .iter()
        .filter(|f| f.name.as_ref().is_some_and(|n| to_generate.contains(n)))
    {
        let package = file.package.as_deref().unwrap_or_default();
        for msg in &file.message_type {
            collect(msg, package, &mut int_fields, &mut all);
        }
    }

    let mut out = HashMap::new();
    // 1. Proto comments.
    for fqn in &all {
        if let Some(c) = ctx.comment(fqn)
            && let Some(m) = PROTO_REF.captures(c)
        {
            let proto_name = &m[1];
            let name = PROTO_ENUM_ALIASES
                .iter()
                .find(|(p, _)| *p == proto_name)
                .map_or(proto_name, |(_, n)| n);
            if let Some(l) = lists
                .iter()
                .find(|l| l.rust_path.ends_with(&format!("::{name}")))
            {
                out.insert(fqn.clone(), l.rust_path.clone());
            }
        }
    }
    // 2. Spec tables.
    let mut object = None;
    for line in spec_md.lines() {
        if let Some(m) = MD_OBJECT.captures(line) {
            object = Some(m[1].to_owned());
            continue;
        }
        let (Some(obj), Some(row)) = (&object, MD_ROW.captures(line)) else {
            continue;
        };
        if !row[2].contains("integer") {
            continue;
        }
        let Some(r) = MD_LIST_REF.captures(&row[3]) else {
            continue;
        };
        let Some(list) = resolve_list(lists, &r[1], &r[2]) else {
            continue;
        };
        if let Some(fqns) = int_fields.get(&format!("{obj}.{}", &row[1]))
            && let [fqn] = fqns.as_slice()
        {
            out.entry(fqn.clone())
                .or_insert_with(|| list.rust_path.clone());
        }
    }
    out.retain(|fqn, _| {
        !NOT_ENUM_FIELDS
            .iter()
            .any(|f| fqn.ends_with(&format!(".{f}")))
    });
    out
}

fn collect(
    msg: &DescriptorProto,
    parent: &str,
    by_name: &mut BTreeMap<String, Vec<String>>,
    all: &mut Vec<String>,
) {
    let name = msg.name.as_deref().unwrap();
    let fqn = format!("{parent}.{name}");
    for f in &msg.field {
        if f.r#type == Some(Type::TYPE_INT32) {
            let field = f.name.as_deref().unwrap();
            let ffqn = format!("{fqn}.{field}");
            by_name
                .entry(format!("{name}.{field}"))
                .or_default()
                .push(ffqn.clone());
            all.push(ffqn);
        }
    }
    for n in &msg.nested_type {
        collect(n, &fqn, by_name, all);
    }
}

/// A list reference: by the anchor of its link, else by title.
fn resolve_list<'a>(lists: &'a [KnownList], title: &str, url: &str) -> Option<&'a KnownList> {
    let anchor = url.rsplit('#').next().unwrap_or_default();
    let norm = |s: &str| {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    };
    lists
        .iter()
        .find(|l| l.anchor == anchor)
        .or_else(|| {
            lists
                .iter()
                .find(|l| norm(&format!("list {}", l.title)) == norm(anchor))
        })
        .or_else(|| lists.iter().find(|l| norm(&l.title) == norm(title)))
        .or_else(|| {
            lists
                .iter()
                .find(|l| norm(&l.title).starts_with(&norm(title)))
        })
}

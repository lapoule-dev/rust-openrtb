//! AdCOM 1.0 specification → `adcom.proto` + `enums.proto` (edition 2023).
//!
//! Every object, attribute, list and list value of the spec is emitted; the
//! completeness test (`tests` below) checks it against the parsed spec.
//!
//! Type mapping, from the attribute's type cell and definition:
//! - `integer` linking a list → that enum (open: unknown values preserved);
//! - `integer` documented as `0 = no, 1 = yes` → `bool` (0/1 in JSON);
//! - `integer` holding a Unix timestamp → `int64`; otherwise `int32`;
//! - `float` → `double`; `object` → the linked object, `ext` → `Ext`;
//! - `… array` → `repeated`; `required`/`recommended`/defaults → comments and `[default]`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;
#[cfg(test)]
use std::path::Path;

use crate::spec::{Attr, List, Object, Spec};

pub const PACKAGE: &str = "com.iabtechlab.adcom.v1";
pub const ENUM_PACKAGE: &str = "com.iabtechlab.adcom.v1.enums";

/// List anchor → enum name. Names follow IABTechLab/adcom-proto, which the
/// official OpenRTB 2.x proto refers to (`com.iabtechlab.adcom.v1.enums.DeviceType`),
/// spelled the Rust way (`ApiFramework`, not `APIFramework`).
const LISTS: &[(&str, &str)] = &[
    ("list_agenttypes", "AgentType"),
    ("list_apiframeworks", "ApiFramework"),
    ("list_auditstatuscodes", "AuditStatusCode"),
    ("list_autorefreshtriggers", "AutoRefreshTrigger"),
    ("list_categorytaxonomies", "CategoryTaxonomy"),
    ("list_clicktypes", "ClickType"),
    ("list_companiontypes", "CompanionType"),
    ("list_connectiontypes", "ConnectionType"),
    ("list_contentcontexts", "ContentContext"),
    ("list_creativeattributes", "CreativeAttribute"),
    (
        "list_creativesubtypesaudiovideo",
        "CreativeSubtypeAudioVideo",
    ),
    ("list_creativesubtypesdisplay", "CreativeSubtypeDisplay"),
    ("list_deliverymethods", "DeliveryMethod"),
    ("list_devicetypes", "DeviceType"),
    ("list_displaycontexttypes", "DisplayContextType"),
    ("list_displayplacementtypes", "DisplayPlacementType"),
    (
        "list_doohmultipliermeasurementmourcetypes",
        "DoohMultiplierMeasurementSourceType",
    ),
    ("list_doohvenuetaxonomies", "DoohVenueTaxonomy"),
    ("list_doohvenuetypes", "DoohVenueType"),
    ("list_eventtrackingmethods", "EventTrackingMethod"),
    ("list_eventtypes", "EventType"),
    ("list_expandabledirections", "ExpandableDirection"),
    ("list_feedtypes", "FeedType"),
    ("list_idmatchmethod", "MatchMethod"),
    ("list_iplocationservices", "LocationService"),
    ("list_linearitymodes", "LinearityMode"),
    ("list_locationtypes", "LocationType"),
    ("list_mediaratings", "MediaRating"),
    ("list_nativedataassettypes", "NativeDataAssetType"),
    ("list_nativeimageassettypes", "NativeImageAssetType"),
    ("list_operatingsystems", "OperatingSystem"),
    ("list_placementpositions", "PlacementPosition"),
    ("list_placementsubtypesvideo", "VideoPlacementSubtype"),
    ("list_plcmtsubtypesvideo", "VideoPlcmtSubtype"),
    ("list_playbackcessationmodes", "PlaybackCessationMode"),
    ("list_playbackmethods", "PlaybackMethod"),
    ("list_poddedupe", "PodDedupe"),
    ("list_podsequence", "PodSequence"),
    ("list_productionqualities", "ProductionQuality"),
    ("list_sizeunits", "SizeUnit"),
    ("list_slotpositioninpod", "SlotPositionInPod"),
    ("list_startdelaymodes", "StartDelayMode"),
    ("user-agent_source", "UserAgentSource"),
    ("list_volumenormalizationmodes", "VolumeNormalizationMode"),
];

/// Lists whose field holds more than the enumerated values (`startdelay` > 0
/// is a delay in seconds): the field stays an integer, the enum names the
/// special values.
const NOT_ENUM_FIELD: &[&str] = &["list_startdelaymodes"];

/// Object anchors whose title is not a type name.
const OBJECT_NAMES: &[(&str, &str)] = &[("object_eids", "Eid"), ("object_eid_uids", "Uid")];

pub fn enum_name(anchor: &str) -> Option<&'static str> {
    LISTS.iter().find(|(a, _)| *a == anchor).map(|(_, n)| *n)
}

fn object_name(o: &Object) -> String {
    OBJECT_NAMES
        .iter()
        .find(|(a, _)| *a == o.anchor)
        .map(|(_, n)| (*n).to_owned())
        .unwrap_or_else(|| o.title.replace(' ', ""))
}

pub struct Generated {
    pub adcom_proto: String,
    pub enums_proto: String,
}

pub fn generate(spec: &Spec) -> Generated {
    Generated {
        adcom_proto: objects_proto(spec),
        enums_proto: enums_proto(spec),
    }
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

fn enums_proto(spec: &Spec) -> String {
    let mut out = header(ENUM_PACKAGE, &[]);
    for list in &spec.lists {
        let name = enum_name(&list.anchor)
            .unwrap_or_else(|| panic!("no enum name for list {}", list.anchor));
        emit_enum(&mut out, name, list);
    }
    out
}

fn emit_enum(out: &mut String, name: &str, list: &List) {
    let prefix = shouty(name);
    comment(
        out,
        "",
        &format!("AdCOM 1.0 List: {}. {}", list.title, list.doc),
    );
    for (v, d) in &list.notes {
        comment(out, "", &format!("{v}: {d}"));
    }
    writeln!(out, "enum {name} {{").unwrap();
    let mut used = HashSet::new();
    if !list.values.iter().any(|v| v.value == 0) {
        writeln!(
            out,
            "  // Not in the spec: the zero value open enums require.\n  {prefix}_UNSPECIFIED = 0;"
        )
        .unwrap();
        used.insert("UNSPECIFIED".to_owned());
    }
    // Open enums must declare 0 first; otherwise keep the spec's order.
    let mut values: Vec<_> = list.values.iter().collect();
    values.sort_by_key(|v| v.value != 0);
    for v in values {
        let base = VALUE_NAMES
            .iter()
            .find(|(e, n, _)| *e == name && *n == v.value)
            .map(|(_, _, n)| (*n).to_owned())
            .unwrap_or_else(|| derive_name(&v.name_source));
        let mut value_name = base.clone();
        let mut n = 2;
        while !used.insert(value_name.clone()) {
            value_name = format!("{base}_{n}");
            n += 1;
        }
        comment(out, "  ", &v.doc);
        writeln!(out, "  {prefix}_{value_name} = {};", v.value).unwrap();
    }
    out.push_str("}\n\n");
}

/// Value names where the one derived from the definition would be unwieldy.
/// Variant names were deliberately NOT taken from iab-specs: its enums
/// disagree with the spec for 54 values (e.g. OperatingSystem 1 = `IOS`,
/// where AdCOM says 1 = "3DS System Software").
const VALUE_NAMES: &[(&str, i32, &str)] = &[
    ("AgentType", 1, "BROWSER_OR_DEVICE"),
    ("AgentType", 2, "IN_APP"),
    ("AgentType", 3, "PERSON"),
    ("CategoryTaxonomy", 1, "CONTENT_CATEGORY_1_0"),
    ("CategoryTaxonomy", 2, "CONTENT_CATEGORY_2_0"),
    ("CategoryTaxonomy", 3, "AD_PRODUCT_1_0"),
    ("CategoryTaxonomy", 4, "AUDIENCE_1_1"),
    ("CategoryTaxonomy", 5, "CONTENT_2_1"),
    ("CategoryTaxonomy", 6, "CONTENT_2_2"),
    ("CategoryTaxonomy", 7, "CONTENT_3_0"),
    ("CategoryTaxonomy", 8, "AD_PRODUCT_2_0"),
    ("CategoryTaxonomy", 9, "CONTENT_3_1"),
    ("CreativeAttribute", 9, "PROVOCATIVE_IMAGERY"),
    ("CreativeAttribute", 10, "EXTREME_ANIMATION"),
    ("CreativeAttribute", 14, "WINDOWS_DIALOG"),
    ("CreativeAttribute", 16, "SKIP_BUTTON"),
    ("CreativeAttribute", 19, "ADVERTISER_QR_CODE"),
    ("CreativeAttribute", 20, "ALPHA_CHANNEL"),
    ("CreativeAttribute", 21, "STATIC"),
    ("CreativeAttribute", 22, "LIMITED_MOTION"),
    ("CreativeAttribute", 23, "FULL_MOTION"),
    ("DisplayContextType", 10, "CONTENT"),
    ("DisplayContextType", 11, "ARTICLE"),
    ("DisplayContextType", 12, "VIDEO"),
    ("DisplayContextType", 13, "AUDIO"),
    ("DisplayContextType", 14, "IMAGE"),
    ("DisplayContextType", 15, "USER_GENERATED"),
    ("DisplayContextType", 20, "SOCIAL"),
    ("DisplayContextType", 21, "EMAIL"),
    ("DisplayContextType", 22, "CHAT_IM"),
    ("DisplayContextType", 30, "PRODUCT"),
    ("DisplayContextType", 31, "APP_STORE"),
    ("DisplayContextType", 32, "PRODUCT_REVIEWS"),
    ("DisplayPlacementType", 1, "IN_FEED"),
    ("DisplayPlacementType", 2, "IN_ATOMIC_UNIT"),
    ("DisplayPlacementType", 3, "OUTSIDE_CORE_CONTENT"),
    ("DisplayPlacementType", 4, "RECOMMENDATION_WIDGET"),
    ("PlaybackCessationMode", 1, "VIDEO_COMPLETION"),
    ("PlaybackCessationMode", 2, "LEAVING_VIEWPORT"),
    ("PlaybackCessationMode", 3, "FLOATING_SLIDER"),
    ("PlaybackMethod", 1, "AUTO_SOUND_ON"),
    ("PlaybackMethod", 2, "AUTO_SOUND_OFF"),
    ("PlaybackMethod", 3, "CLICK_SOUND_ON"),
    ("PlaybackMethod", 4, "MOUSE_OVER_SOUND_ON"),
    ("PlaybackMethod", 5, "VIEWPORT_SOUND_ON"),
    ("PlaybackMethod", 6, "VIEWPORT_SOUND_OFF"),
    ("PlaybackMethod", 7, "CONTINUOUS"),
    ("PlaybackMethod", 8, "PAUSE_SOUND_ON"),
    ("PlaybackMethod", 9, "PAUSE_SOUND_OFF"),
    ("PlaybackMethod", 10, "IDLE_SOUND_ON"),
    ("PlaybackMethod", 11, "IDLE_SOUND_OFF"),
    ("ConnectionType", 3, "CELLULAR_UNKNOWN"),
    ("ConnectionType", 4, "CELLULAR_2G"),
    ("ConnectionType", 5, "CELLULAR_3G"),
    ("ConnectionType", 6, "CELLULAR_4G"),
    ("ConnectionType", 7, "CELLULAR_5G"),
    ("FeedType", 1, "MUSIC_SERVICE"),
    ("FeedType", 2, "BROADCAST"),
    ("FeedType", 3, "PODCAST"),
    ("FeedType", 4, "CATCH_UP_RADIO"),
    ("FeedType", 5, "WEB_RADIO"),
    ("FeedType", 6, "VIDEO_GAME"),
    ("FeedType", 7, "TEXT_TO_SPEECH"),
    ("PodSequence", -1, "LAST"),
    ("PodSequence", 0, "ANY"),
    ("PodSequence", 1, "FIRST"),
    ("SlotPositionInPod", -1, "LAST"),
    ("SlotPositionInPod", 0, "ANY"),
    ("SlotPositionInPod", 1, "FIRST"),
    ("SlotPositionInPod", 2, "FIRST_OR_LAST"),
    ("SizeUnit", 1, "DIPS"),
    ("VolumeNormalizationMode", 1, "AVERAGE_VOLUME"),
    ("VolumeNormalizationMode", 2, "PEAK_VOLUME"),
    ("VolumeNormalizationMode", 3, "LOUDNESS"),
    ("VolumeNormalizationMode", 4, "CUSTOM"),
    ("PodDedupe", 1, "ADOMAIN"),
    ("PodDedupe", 2, "IAB_CATEGORY"),
    ("PodDedupe", 3, "CREATIVE_ID"),
    ("PodDedupe", 4, "MEDIAFILE_URL"),
    ("PodDedupe", 5, "NONE"),
    ("UserAgentSource", 1, "CLIENT_HINTS_LOW_ENTROPY"),
    ("UserAgentSource", 2, "CLIENT_HINTS_HIGH_ENTROPY"),
    ("UserAgentSource", 3, "USER_AGENT_HEADER"),
];

/// "Mobile/Tablet - General" → `MOBILE_TABLET_GENERAL`; "Pending Audit: An
/// audit has…" → `PENDING_AUDIT`.
fn derive_name(doc: &str) -> String {
    let head = doc.split(" http").next().unwrap_or(doc);
    let head = head.split([':', ';', ',']).next().unwrap_or(head);
    let words: Vec<String> = head
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(8)
        .map(str::to_ascii_uppercase)
        .collect();
    let name = words.join("_");
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        format!("V_{name}")
    } else {
        name
    }
}

/// `ApiFramework` / `Simid1_1` → `API_FRAMEWORK` / `SIMID1_1`.
fn shouty(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() && i > 0 {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev.is_lowercase() || prev.is_ascii_digit() || (prev.is_uppercase() && next_lower) {
                out.push('_');
            }
        }
        out.push(c.to_ascii_uppercase());
    }
    out.replace("__", "_")
}

// ---------------------------------------------------------------------------
// Objects
// ---------------------------------------------------------------------------

fn objects_proto(spec: &Spec) -> String {
    let mut out = header(
        PACKAGE,
        &[
            "com/iabtechlab/adcom/v1/enums.proto",
            "google/protobuf/struct.proto",
        ],
    );
    let names: BTreeMap<&str, String> = spec
        .objects
        .iter()
        .map(|o| (o.anchor.as_str(), object_name(o)))
        .collect();
    for o in spec.objects.iter().filter(|o| !o.is_abstract) {
        let mut attrs: Vec<Attr> = Vec::new();
        if let Some(parent) = &o.derived_from {
            let parent = spec
                .objects
                .iter()
                .find(|p| &p.anchor == parent)
                .expect("abstract parent");
            attrs.extend(parent.attrs.iter().cloned());
        }
        attrs.extend(o.attrs.iter().cloned());
        let name = &names[o.anchor.as_str()];
        comment(
            &mut out,
            "",
            &format!("AdCOM 1.0 Object: {}. {}", o.title, o.doc),
        );
        writeln!(out, "message {name} {{").unwrap();
        let mut number = 0;
        let mut has_ext = false;
        for a in &attrs {
            if a.name == "ext" {
                has_ext = true;
                continue;
            }
            number += 1;
            emit_field(&mut out, a, number, &names, spec);
        }
        if has_ext {
            out.push_str("\n  // Optional vendor-specific extensions.\n  Ext ext = 99;\n\n  message Ext {\n    extensions 500 to max;\n  }\n");
        }
        out.push_str("}\n\n");
    }
    out
}

fn emit_field(
    out: &mut String,
    a: &Attr,
    number: u32,
    names: &BTreeMap<&str, String>,
    spec: &Spec,
) {
    let ty = a.ty.as_str();
    let repeated = ty.contains("array");
    let required = ty.contains("required");
    let recommended = ty.contains("recommended");
    let deprecated =
        ty.contains("deprecated") || a.doc.to_ascii_lowercase().starts_with("deprecated");
    let default = ty.split("default").nth(1).map(|d| {
        d.trim()
            .trim_matches(|c| c == ';' || c == '"')
            .trim()
            .to_owned()
    });
    let list = a.links.iter().find(|l| enum_name(l).is_some()).cloned();
    // Without a link, an object attribute names its type in backticks
    // (`Producer` (Section 3.2.17)) or by its own name (`producer`).
    let object = a
        .links
        .iter()
        .find(|l| names.contains_key(l.as_str()))
        .cloned()
        .or_else(|| {
            names
                .iter()
                .find(|(_, n)| a.doc.contains(&format!("`{n}`")) || n.eq_ignore_ascii_case(&a.name))
                .map(|(anchor, _)| (*anchor).to_owned())
        });
    let doc_lower = a.doc.to_ascii_lowercase();

    let base = ty.split([';', ',']).next().unwrap_or(ty).trim();
    let (proto_ty, default_lit) = if base.contains("object") {
        let target = object
            .as_ref()
            .map(|o| names[o.as_str()].clone())
            .unwrap_or_else(|| "google.protobuf.Struct".to_owned());
        (target, None)
    } else if base.contains("integer") {
        match &list {
            Some(l) if !NOT_ENUM_FIELD.contains(&l.as_str()) => {
                let e = enum_name(l).unwrap();
                let lit = default.as_ref().and_then(|d| enum_default(spec, l, e, d));
                (format!("{ENUM_PACKAGE}.{e}"), lit)
            }
            _ if is_flag(&doc_lower) => (
                "bool".to_owned(),
                default.as_ref().map(|d| {
                    if d == "1" {
                        "true".into()
                    } else {
                        "false".into()
                    }
                }),
            ),
            _ if doc_lower.contains("timestamp") || doc_lower.contains("since the epoch") => {
                ("int64".to_owned(), default.clone())
            }
            _ => ("int32".to_owned(), default.clone()),
        }
    } else if base.contains("float") {
        ("double".to_owned(), default.clone())
    } else {
        (
            "string".to_owned(),
            default.as_ref().map(|d| format!("{d:?}")),
        )
    };

    let mut doc = a.doc.clone();
    if required {
        doc.push_str(" REQUIRED by the AdCOM specification.");
    } else if recommended {
        doc.push_str(" RECOMMENDED by the AdCOM specification.");
    }
    out.push('\n');
    comment(out, "  ", &doc);
    let mut opts = Vec::new();
    if let Some(d) = default_lit.filter(|_| !repeated) {
        opts.push(format!("default = {d}"));
    }
    if deprecated {
        opts.push("deprecated = true".to_owned());
    }
    let opts = if opts.is_empty() {
        String::new()
    } else {
        format!(" [{}]", opts.join(", "))
    };
    let label = if repeated { "repeated " } else { "" };
    writeln!(out, "  {label}{proto_ty} {} = {number}{opts};", a.name).unwrap();
}

fn is_flag(doc: &str) -> bool {
    let d = doc.replace(' ', "");
    (d.contains("0=no") && d.contains("1=yes"))
        || d.contains("1=yes,0=no")
        || d.contains("where0=") && d.contains("1=")
}

fn enum_default(spec: &Spec, list_anchor: &str, enum_name: &str, default: &str) -> Option<String> {
    let value: i32 = default.parse().ok()?;
    // Value names are resolved from the generated enum text to stay consistent.
    let list = spec.lists.iter().find(|l| l.anchor == list_anchor)?;
    list.values
        .iter()
        .any(|v| v.value == value)
        .then(|| format!("__ENUM_DEFAULT__{enum_name}:{value}"))
}

/// Replaces `__ENUM_DEFAULT__Enum:3` placeholders by the value names of `enums_proto`.
pub fn resolve_enum_defaults(adcom_proto: &str, enums_proto: &str) -> String {
    let mut names: HashMap<(String, i32), String> = HashMap::new();
    let mut current = String::new();
    for line in enums_proto.lines() {
        if let Some(rest) = line.strip_prefix("enum ") {
            current = rest.trim_end_matches(" {").to_owned();
        } else if let Some((name, value)) = line
            .trim()
            .strip_suffix(';')
            .and_then(|l| l.split_once(" = "))
            && let Ok(v) = value.parse()
        {
            names.insert((current.clone(), v), name.to_owned());
        }
    }
    let mut out = String::with_capacity(adcom_proto.len());
    let mut rest = adcom_proto;
    while let Some(i) = rest.find("__ENUM_DEFAULT__") {
        out.push_str(&rest[..i]);
        let tail = &rest[i + "__ENUM_DEFAULT__".len()..];
        let end = tail.find([']', ',']).unwrap();
        let (e, v) = tail[..end].split_once(':').unwrap();
        out.push_str(&names[&(e.to_owned(), v.parse().unwrap())]);
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------

fn header(package: &str, imports: &[&str]) -> String {
    let mut out = format!(
        "// @generated by openrtb-codegen from the AdCOM 1.0 specification\n\
         // (reference/AdCOM/AdCOM v1.0 FINAL.md). DO NOT EDIT.\n\n\
         edition = \"2023\";\n\npackage {package};\n\n"
    );
    for i in imports {
        writeln!(out, "import \"{i}\";").unwrap();
    }
    if !imports.is_empty() {
        out.push('\n');
    }
    out
}

/// Writes `text` as `//` comment lines wrapped at ~80 columns.
fn comment(out: &mut String, indent: &str, text: &str) {
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && indent.len() + 3 + line.len() + word.len() > 80 {
            writeln!(out, "{indent}// {line}").unwrap();
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        writeln!(out, "{indent}// {line}").unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn names() {
        assert_eq!(shouty("ApiFramework"), "API_FRAMEWORK");
        assert_eq!(shouty("Simid1_1"), "SIMID1_1");
        assert_eq!(shouty("DoohVenueType"), "DOOH_VENUE_TYPE");
        assert_eq!(
            derive_name("Mobile/Tablet - General"),
            "MOBILE_TABLET_GENERAL"
        );
        assert_eq!(derive_name("3rd party"), "V_3RD_PARTY");
    }

    /// Every object, attribute, list and value of the spec is in the committed protos.
    #[test]
    fn adcom_is_complete() {
        let root = root();
        let md = std::fs::read_to_string(root.join("reference/AdCOM/AdCOM v1.0 FINAL.md")).unwrap();
        let spec = crate::spec::parse(&md);
        let adcom = std::fs::read_to_string(root.join("proto/com/iabtechlab/adcom/v1/adcom.proto"))
            .unwrap();
        let enums = std::fs::read_to_string(root.join("proto/com/iabtechlab/adcom/v1/enums.proto"))
            .unwrap();

        assert_eq!(
            spec.objects.len(),
            46,
            "AdCOM 1.0 has 45 objects + 1 abstract class"
        );
        assert_eq!(spec.lists.len(), 44, "AdCOM 1.0 has 44 lists");
        let mut fields = 0;
        for o in spec.objects.iter().filter(|o| !o.is_abstract) {
            let name = object_name(o);
            let body =
                message_body(&adcom, &name).unwrap_or_else(|| panic!("missing message {name}"));
            let inherited = o
                .derived_from
                .as_ref()
                .map(|p| {
                    spec.objects
                        .iter()
                        .find(|x| &x.anchor == p)
                        .unwrap()
                        .attrs
                        .clone()
                })
                .unwrap_or_default();
            for a in inherited.iter().chain(&o.attrs) {
                let decl = if a.name == "ext" {
                    "Ext ext = 99;".to_owned()
                } else {
                    format!(" {} = ", a.name)
                };
                assert!(body.contains(&decl), "{name}.{} missing", a.name);
                fields += 1;
            }
        }
        assert!(fields >= 376, "only {fields} attributes");
        for l in &spec.lists {
            let e = enum_name(&l.anchor).unwrap();
            let body = message_body(&enums, e).unwrap_or_else(|| panic!("missing enum {e}"));
            for v in &l.values {
                assert!(
                    body.contains(&format!(" = {};", v.value)),
                    "{e} value {} missing",
                    v.value
                );
            }
        }
    }

    fn message_body<'a>(proto: &'a str, name: &str) -> Option<&'a str> {
        let start = proto
            .find(&format!("message {name} {{"))
            .or_else(|| proto.find(&format!("enum {name} {{")))?;
        let body = &proto[start..];
        Some(&body[..body.find("\n}\n").unwrap_or(body.len())])
    }
}

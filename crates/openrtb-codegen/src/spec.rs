//! Parser for IAB specification markdown (AdCOM, OpenRTB 3.0): the object and
//! list sections, whose tables are written in HTML.
//!
//! ```text
//! ### Object:  Ad <a name="object_ad"></a>
//! <table><tr><td><code>id</code></td><td>string; required</td><td>…</td></tr>…</table>
//!
//! ### List:  Device Types <a name="list_devicetypes"></a>
//! <table><tr><td>1</td><td>Mobile/Tablet - General</td></tr>…</table>
//! ```

use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug)]
pub struct Spec {
    pub objects: Vec<Object>,
    pub lists: Vec<List>,
}

#[derive(Debug)]
pub struct Object {
    pub title: String,
    pub anchor: String,
    pub doc: String,
    pub is_abstract: bool,
    /// Anchor of the abstract class this object derives from.
    pub derived_from: Option<String>,
    pub attrs: Vec<Attr>,
}

#[derive(Debug, Clone)]
pub struct Attr {
    pub name: String,
    /// Normalized type cell, e.g. `string array; recommended`.
    pub ty: String,
    pub doc: String,
    /// Anchors linked from the definition, in order (`object_…`, `list_…`).
    pub links: Vec<String>,
}

#[derive(Debug)]
pub struct List {
    pub title: String,
    pub anchor: String,
    pub doc: String,
    pub values: Vec<ListValue>,
    /// Rows that are not a single integer (`500+`, `>0`): documented, not enumerated.
    pub notes: Vec<(String, String)>,
}

#[derive(Debug)]
pub struct ListValue {
    pub value: i32,
    pub doc: String,
    /// The definition column, from which the value name is derived.
    pub name_source: String,
}

static HEADING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^\s*#{2,4}\s+(Object|List|Abstract Class):\s+(.+?)\s*<a name="([^"]+)">"#)
        .unwrap()
});
static ANY_HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*#{1,4}\s").unwrap());
static ROW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<tr>(.*?)</tr>").unwrap());
static CELL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<td[^>]*>(.*?)</td>").unwrap());
static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r##"href="[^"#]*#([^"]+)""##).unwrap());
static DERIVED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*Derived from:\*\s*\[[^\]]*\]\(#([^)]+)\)").unwrap());
static MD_ANCHOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\]\([^)#]*#([^)]+)\)").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]+>").unwrap());
static MD_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\([^)]*\)").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

pub fn parse(markdown: &str) -> Spec {
    let heads: Vec<_> = HEADING
        .captures_iter(markdown)
        .map(|c| {
            (
                c.get(0).unwrap(),
                c[1].to_owned(),
                c[2].to_owned(),
                c[3].to_owned(),
            )
        })
        .collect();
    let mut objects = Vec::new();
    let mut lists = Vec::new();
    for (whole, kind, title, anchor) in heads {
        let rest = &markdown[whole.end()..];
        let end = ANY_HEADING.find(rest).map_or(rest.len(), |m| m.start());
        let section = &rest[..end];
        let table_start = section.find("<table").unwrap_or(section.len());
        let intro = &section[..table_start];
        let mut rows: Vec<Vec<&str>> = ROW
            .captures_iter(section)
            .map(|r| {
                CELL.captures_iter(r.get(1).unwrap().as_str())
                    .map(|c| c.get(1).unwrap().as_str())
                    .collect()
            })
            .skip(1) // header row
            .collect();
        if rows.is_empty() {
            // Some sections use markdown tables: `| a | b | c |`, header, `| --- |`.
            rows = section
                .lines()
                .map(str::trim)
                .filter(|l| l.starts_with('|') && !l.starts_with("| ---") && !l.starts_with("|---"))
                .skip(1)
                .map(|l| l.trim_matches('|').split(" | ").map(str::trim).collect())
                .collect();
        }
        let intro = if intro.contains("\n|") {
            &intro[..intro.find("\n|").unwrap()]
        } else {
            intro
        };
        match kind.as_str() {
            "List" => {
                let mut values = Vec::new();
                let mut notes = Vec::new();
                // In `Value | Name | Definition` tables the middle column is a
                // short name (ID Match Methods); in `Value | Class | Definition`
                // it is a category shared by several rows (Feed Types).
                let middles: Vec<String> = rows
                    .iter()
                    .filter(|r| r.len() == 3)
                    .map(|r| text(r[1]))
                    .collect();
                let middle_is_name = !middles.is_empty()
                    && middles
                        .iter()
                        .all(|m| !m.is_empty() && m.split_whitespace().count() <= 4)
                    && middles
                        .iter()
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                        == middles.len();
                for row in rows.iter().filter(|r| r.len() >= 2) {
                    let v = text(row[0]);
                    // `Value | Definition`, or `Value | Class | Definition`.
                    let doc = row[1..]
                        .iter()
                        .map(|c| text(c))
                        .collect::<Vec<_>>()
                        .join(": ");
                    match v.parse::<i32>() {
                        Ok(value) => {
                            let name_col = if middle_is_name && row.len() == 3 {
                                1
                            } else {
                                row.len() - 1
                            };
                            values.push(ListValue {
                                value,
                                doc,
                                name_source: text(row[name_col]),
                            })
                        }
                        Err(_) => notes.push((v, doc)),
                    }
                }
                lists.push(List {
                    title: text(&title),
                    anchor,
                    doc: prose(intro),
                    values,
                    notes,
                });
            }
            _ => {
                let attrs = rows
                    .iter()
                    .filter(|r| r.len() >= 3)
                    .map(|r| Attr {
                        name: text(r[0]).trim_matches('`').to_owned(),
                        ty: text(r[1]).to_lowercase(),
                        doc: text(r[2]),
                        links: LINK
                            .captures_iter(r[2])
                            .chain(MD_ANCHOR.captures_iter(r[2]))
                            .map(|c| c[1].to_owned())
                            .collect(),
                    })
                    .collect();
                objects.push(Object {
                    title: text(&title),
                    is_abstract: kind == "Abstract Class",
                    derived_from: DERIVED.captures(intro).map(|c| c[1].to_owned()),
                    doc: prose(&DERIVED.replace(intro, "")),
                    anchor,
                    attrs,
                });
            }
        }
    }
    let mut spec = Spec { objects, lists };
    normalize_links(&mut spec, markdown);
    spec
}

/// Markdown tables link with GitHub heading slugs (`#list--device-types-`);
/// map them to the declared `<a name>` anchors (`list_devicetypes`).
fn normalize_links(spec: &mut Spec, markdown: &str) {
    let mut slugs = std::collections::HashMap::new();
    for c in HEADING.captures_iter(markdown) {
        let heading = format!("{}: {}", &c[1], &c[2]);
        slugs.insert(slug(&heading), c[3].to_owned());
        // Doubled spaces after the colon are common in the spec.
        slugs.insert(slug(&format!("{}:  {}", &c[1], &c[2])), c[3].to_owned());
    }
    let known: std::collections::HashSet<String> = spec
        .objects
        .iter()
        .map(|o| o.anchor.clone())
        .chain(spec.lists.iter().map(|l| l.anchor.clone()))
        .collect();
    for o in &mut spec.objects {
        for a in &mut o.attrs {
            for link in &mut a.links {
                if !known.contains(link.as_str())
                    && let Some(anchor) = slugs.get(link.trim_matches('-'))
                {
                    *link = anchor.clone();
                }
            }
        }
    }
}

/// GitHub heading slug, without leading/trailing dashes.
fn slug(heading: &str) -> String {
    let s: String = heading
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect();
    s.trim_matches('-').to_owned()
}

/// Cell/HTML fragment → plain single-line text.
pub fn text(html: &str) -> String {
    let s = html.replace("<br/>", " ").replace("<br>", " ");
    let s = TAG.replace_all(&s, "");
    let s = unescape(&s);
    SPACES.replace_all(s.trim(), " ").into_owned()
}

/// Markdown intro paragraph → plain text.
fn prose(md: &str) -> String {
    let s = MD_LINK.replace_all(md, "$1");
    text(&s.replace('`', ""))
}

fn unescape(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace('\u{a0}', " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

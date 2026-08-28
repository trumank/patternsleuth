//! Compact, column aligned rendering of scan results.
//!
//! Almost every resolver returns a single address, and `{:#x?}` spends three
//! lines on one (`GEngine(\n    0x14a516b10,\n)`). Rather than teach every
//! resolution type how to print itself, the pretty output is read back into a
//! tree here and laid out again: a value is printed on one line whenever it
//! fits the space left beside its name, and only the parts that don't fit stay
//! broken across lines.

use std::fmt::{Debug, Display};

/// Widest a name column is allowed to grow before values start overflowing it.
const NAME_MAX: usize = 45;

/// Narrowest value column worth laying out for.
const VALUE_MIN: usize = 40;

/// Width to lay out for when output isn't going to a terminal.
const DEFAULT_WIDTH: usize = 200;

/// Width available for output, from the terminal when there is one.
pub fn term_width() -> usize {
    console::Term::stdout()
        .size_checked()
        .map(|(_, w)| w as usize)
        .unwrap_or(DEFAULT_WIDTH)
}

/// Space left for values once the name column and its padding are taken.
pub fn value_width(name_width: usize) -> usize {
    term_width().saturating_sub(name_width + 4).max(VALUE_MIN)
}

/// A value's `{:#x?}` output, read back so it can be laid out again.
///
/// Rust's pretty debug output is regular enough to parse: a group is a line
/// ending in an open bracket, its contents indented under it, and the closing
/// bracket on its own line. Everything else is a leaf and prints as it is.
enum Node {
    Leaf(String),
    Group {
        open: String,
        children: Vec<Node>,
        close: String,
    },
}

type Lines<'a> = std::iter::Peekable<std::str::Lines<'a>>;

fn parse(lines: &mut Lines<'_>) -> Option<Node> {
    let line = lines.next()?.trim().to_string();
    if !line.ends_with(['(', '{', '[']) {
        return Some(Node::Leaf(line));
    }
    let mut children = vec![];
    while lines
        .peek()
        .is_some_and(|l| !l.trim_start().starts_with([')', '}', ']']))
    {
        children.push(parse(lines)?);
    }
    Some(Node::Group {
        open: line,
        children,
        close: lines.next().unwrap_or_default().trim().to_string(),
    })
}

impl Node {
    /// The whole value on one line, as the non-alternate `{:?}` would have it.
    fn flat(&self) -> String {
        match self {
            Node::Leaf(text) => text.clone(),
            Node::Group {
                open,
                children,
                close,
            } => {
                if children.is_empty() {
                    return format!("{open}{close}");
                }
                // braces get inner padding (`{ a: 1 }`), brackets don't (`[1]`)
                let pad = if open.ends_with('{') { " " } else { "" };
                let inner = children
                    .iter()
                    .map(|c| c.flat().trim_end_matches(',').to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{open}{pad}{inner}{pad}{close}")
            }
        }
    }

    /// Lay out at `width`, breaking only the groups that don't fit.
    fn render(&self, indent: usize, width: usize, out: &mut String) {
        let flat = self.flat();
        if indent + flat.chars().count() <= width {
            push_line(out, indent, &flat);
            return;
        }
        match self {
            Node::Leaf(text) => push_line(out, indent, text),
            Node::Group {
                open,
                children,
                close,
            } => {
                push_line(out, indent, open);
                if !self.render_packed(indent + 4, width, out) {
                    for child in children {
                        child.render(indent + 4, width, out);
                    }
                }
                push_line(out, indent, close);
            }
        }
    }

    /// Lay a list of plain values out as a grid, several to a line.
    ///
    /// A long list of one value per line is mostly empty space, and lists like
    /// custom versions or AES keys are uniform enough to read across as well as
    /// down. Struct fields aren't, so they're left one per line.
    fn render_packed(&self, indent: usize, width: usize, out: &mut String) -> bool {
        let Node::Group { children, .. } = self else {
            return false;
        };
        let entries = children
            .iter()
            .map(|child| match child {
                Node::Leaf(text) if !is_field(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>();
        let Some(entries) = entries.filter(|e| e.len() > 1) else {
            return false;
        };

        let entry_width = entries.iter().map(|e| e.chars().count()).max().unwrap_or(0);
        let columns = width.saturating_sub(indent).saturating_add(1) / (entry_width + 1);
        if columns < 2 {
            return false;
        }
        for row in entries.chunks(columns) {
            let line = row
                .iter()
                .map(|entry| format!("{entry:entry_width$}"))
                .collect::<Vec<_>>()
                .join(" ");
            push_line(out, indent, line.trim_end());
        }
        true
    }

    /// Drop the type name the row is already labelled with.
    ///
    /// `GEngine` next to `GEngine(0x14a516b10)` says nothing, and dropping it
    /// leaves the address alone in the column where it can be read down.
    fn strip_name(self, name: &str) -> Node {
        match self {
            Node::Leaf(text) => Node::Leaf(match strip_prefix(&text, name) {
                // a leaf carries its own parens: `EngineVersion(5.8)` -> `5.8`
                Some(rest) => unwrap_parens(rest).to_string(),
                None => text,
            }),
            Node::Group {
                open,
                mut children,
                close,
            } => {
                let Some(open) = strip_prefix(&open, name).map(str::to_string) else {
                    return Node::Group {
                        open,
                        children,
                        close,
                    };
                };
                // a newtype wraps its one value in parens: unwrap those too
                if open == "(" && close == ")" && children.len() == 1 {
                    return match children.pop().unwrap() {
                        Node::Leaf(text) => Node::Leaf(text.trim_end_matches(',').to_string()),
                        Node::Group {
                            open,
                            children,
                            close,
                        } => Node::Group {
                            open,
                            children,
                            close: close.trim_end_matches(',').to_string(),
                        },
                    };
                }
                Node::Group {
                    open,
                    children,
                    close,
                }
            }
        }
    }
}

/// `text` without a leading `name`, as long as `name` isn't part of a longer word.
fn strip_prefix<'t>(text: &'t str, name: &str) -> Option<&'t str> {
    let rest = text.strip_prefix(name)?;
    (!rest.is_empty() && !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
        .then(|| rest.trim_start())
}

/// Whether `text` is a `name: value` struct field rather than a bare value.
fn is_field(text: &str) -> bool {
    text.split_once(": ").is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')
    })
}

fn unwrap_parens(text: &str) -> &str {
    text.strip_prefix('(')
        .and_then(|t| t.strip_suffix(')'))
        .unwrap_or(text)
}

fn push_line(out: &mut String, indent: usize, line: &str) {
    for _ in 0..indent {
        out.push(' ');
    }
    out.push_str(line);
    out.push('\n');
}

/// A resolution laid out to fit `width`, without the type name `name` repeats.
pub fn resolution(name: &str, value: &dyn Debug, width: usize) -> String {
    let text = format!("{value:#x?}");
    let Some(node) = parse(&mut text.lines().peekable()) else {
        return text;
    };
    let mut out = String::new();
    node.strip_name(name).render(0, width, &mut out);
    out.pop(); // trailing newline
    out
}

/// A resolution on one line, for grids that lay out rows of them.
pub fn resolution_flat(name: &str, value: &dyn Debug) -> String {
    resolution(name, value, usize::MAX)
}

/// A resolve error, without the context entry naming the resolver itself.
///
/// [`Display`] gives `context: context: message` where the outermost context is
/// always the resolver being reported, which the row is already labelled with.
pub fn error(name: &str, err: &dyn Display) -> String {
    let text = err.to_string();
    match text.strip_prefix(&format!("{name}: ")) {
        Some(rest) => rest.to_string(),
        None => text,
    }
}

/// Lay out `rows` as a name column and a value column.
///
/// Values run to as many lines as they need; the extra lines line up under the
/// first so a value reads as one block.
pub fn columns<'r>(rows: impl IntoIterator<Item = (&'r str, &'r str)>) -> String {
    let rows = rows.into_iter().collect::<Vec<_>>();
    let name_width = rows
        .iter()
        .map(|(name, _)| console::measure_text_width(name))
        .max()
        .unwrap_or(0)
        .min(NAME_MAX);

    let mut out = String::new();
    for (name, value) in rows {
        let pad = name_width.saturating_sub(console::measure_text_width(name));
        let mut lines = value.lines();
        let first = lines.next().unwrap_or_default();
        out.push_str(&format!("  {name}{:pad$}  {}\n", "", first.trim_end()));
        for line in lines {
            out.push_str(&format!("  {:name_width$}  {}\n", "", line.trim_end()));
        }
    }
    out.pop(); // trailing newline
    out
}

#[cfg(test)]
#[allow(dead_code)] // fixture fields exist to be printed by the derived Debug
mod test {
    use super::*;

    #[derive(Debug)]
    struct GEngine(u64);

    #[derive(Debug)]
    struct FFrameStepViaExec {
        step: u64,
        step_explicit_property: u64,
    }

    #[derive(Debug)]
    struct InternalProjectName {
        internal_project_name: u64,
        compiled_in_project_name: Option<String>,
    }

    #[derive(Debug)]
    enum BuildConfiguration {
        Shipping,
    }

    struct AESKeys(Vec<&'static str>);
    impl Debug for AESKeys {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "AESKeys ")?;
            f.debug_list().entries(self.0.iter()).finish()
        }
    }

    #[test]
    fn singleton_is_just_an_address() {
        assert_eq!(
            resolution("GEngine", &GEngine(0x14a516b10), 80),
            "0x14a516b10"
        );
    }

    #[test]
    fn struct_fits_on_one_line() {
        let value = FFrameStepViaExec {
            step: 0x1402c3860,
            step_explicit_property: 0x1402c3890,
        };
        assert_eq!(
            resolution("FFrameStepViaExec", &value, 80),
            "{ step: 0x1402c3860, step_explicit_property: 0x1402c3890 }"
        );
    }

    #[test]
    fn nested_value_is_flattened() {
        let value = InternalProjectName {
            internal_project_name: 0x141cf1470,
            compiled_in_project_name: Some("Kholat".into()),
        };
        assert_eq!(
            resolution("InternalProjectName", &value, 120),
            r#"{ internal_project_name: 0x141cf1470, compiled_in_project_name: Some("Kholat") }"#
        );
    }

    #[test]
    fn only_what_doesnt_fit_is_broken_up() {
        let value = InternalProjectName {
            internal_project_name: 0x141cf1470,
            compiled_in_project_name: Some("Kholat".into()),
        };
        assert_eq!(
            resolution("InternalProjectName", &value, 60),
            r#"{
    internal_project_name: 0x141cf1470,
    compiled_in_project_name: Some("Kholat"),
}"#
        );
    }

    #[test]
    fn unnamed_values_are_left_alone() {
        assert_eq!(
            resolution("BuildConfiguration", &BuildConfiguration::Shipping, 80),
            "Shipping"
        );
    }

    #[test]
    fn lists_lose_their_name_too() {
        assert_eq!(resolution("AESKeys", &AESKeys(vec![]), 80), "[]");
        assert_eq!(
            resolution("AESKeys", &AESKeys(vec!["0xaa", "0xbb"]), 80),
            r#"["0xaa", "0xbb"]"#
        );
        assert_eq!(
            resolution("AESKeys", &AESKeys(vec!["0xaa", "0xbb"]), 10),
            r#"[
    "0xaa",
    "0xbb",
]"#
        );
    }

    #[test]
    fn long_lists_are_laid_out_as_a_grid() {
        assert_eq!(
            resolution(
                "AESKeys",
                &AESKeys(vec!["0xaa", "0xbbbb", "0xcc", "0xdd"]),
                30
            ),
            r#"[
    "0xaa",   "0xbbbb",
    "0xcc",   "0xdd",
]"#
        );
    }

    #[test]
    fn struct_fields_stay_one_per_line() {
        let value = FFrameStepViaExec {
            step: 0x1402c3860,
            step_explicit_property: 0x1402c3890,
        };
        assert_eq!(
            resolution("FFrameStepViaExec", &value, 40),
            r#"{
    step: 0x1402c3860,
    step_explicit_property: 0x1402c3890,
}"#
        );
    }

    #[test]
    fn name_of_another_type_is_kept() {
        assert_eq!(
            resolution("GNatives", &GEngine(0x14a516b10), 80),
            "GEngine(0x14a516b10)"
        );
    }

    #[test]
    fn error_drops_its_own_name() {
        assert_eq!(
            error("GEngine", &"GEngine: expected at least one value"),
            "expected at least one value"
        );
        assert_eq!(
            error("GEngine", &"FFrameStep: expected at least one value"),
            "FFrameStep: expected at least one value"
        );
    }

    #[test]
    fn values_line_up_under_their_first_line() {
        assert_eq!(
            columns([("GEngine", "0x14a516b10"), ("AESKeys", "[\n    0xaa,\n]")]),
            "  GEngine  0x14a516b10\n  AESKeys  [\n               0xaa,\n           ]"
        );
    }
}

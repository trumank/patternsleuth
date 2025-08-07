use std::{collections::HashMap, fmt::Write};

type Result<T, E = std::fmt::Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Id {
    String(String),
    Html(XmlNode),
}
impl Default for Id {
    fn default() -> Self {
        Self::String("".into())
    }
}
impl<S> From<S> for Id
where
    S: Into<String>,
{
    fn from(value: S) -> Self {
        Self::String(value.into())
    }
}

#[derive(Default, Debug, Clone)]
pub struct Attributes {
    pub attributes: HashMap<Id, Id>,
}

#[derive(Default, Debug, Clone)]
pub struct GraphBase {
    pub attributes: Attributes,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub subgraphs: Vec<Subgraph>,
    pub graph_attributes: Attributes,
    pub node_attributes: Attributes,
    pub edge_attributes: Attributes,
}

#[derive(Default, Debug, Clone)]
pub struct Graph {
    pub base: GraphBase,
    pub graph_type: String,
}

#[derive(Default, Debug, Clone)]
pub struct Subgraph {
    pub id: Option<String>,
    pub base: GraphBase,
}

#[derive(Default, Debug, Clone)]
pub struct Node {
    pub id: String,
    pub attributes: Attributes,
}

#[derive(Default, Debug, Clone)]
pub struct Edge {
    pub a: String,
    pub a_compass: Option<String>,
    pub b: String,
    pub b_compass: Option<String>,
    pub attributes: Attributes,
}

impl Id {
    fn write<S: Write>(&self, s: &mut S) -> Result<()> {
        match self {
            Id::String(string) => {
                write!(s, r#""{}""#, string.replace("\"", "\\\""))?;
            }
            Id::Html(xml) => {
                write!(s, "<")?;
                xml.write(s)?;
                write!(s, ">")?;
            }
        }
        Ok(())
    }
}

impl Graph {
    pub fn new(graph_type: impl Into<String>) -> Self {
        Self {
            base: Default::default(),
            graph_type: graph_type.into(),
        }
    }
    pub fn write<S: Write>(&self, s: &mut S) -> Result<()> {
        writeln!(s, "{}", self.graph_type)?;
        writeln!(s, "{{")?;
        self.base.write(s)?;
        writeln!(s, "}}")?;
        Ok(())
    }
}

impl GraphBase {
    fn write<S: Write>(&self, s: &mut S) -> Result<()> {
        self.attributes.write_as_statements(s)?;
        self.write_attributes_for(s, "graph", &self.graph_attributes)?;
        self.write_attributes_for(s, "node", &self.node_attributes)?;
        self.write_attributes_for(s, "edge", &self.edge_attributes)?;
        self.nodes.iter().try_for_each(|i| i.write(s))?;
        self.edges.iter().try_for_each(|i| i.write(s))?;
        self.subgraphs.iter().try_for_each(|i| i.write(s))?;
        Ok(())
    }
    fn write_attributes_for<S: Write>(
        &self,
        s: &mut S,
        label: &str,
        attributes: &Attributes,
    ) -> Result<()> {
        if !attributes.attributes.is_empty() {
            write!(s, "{label} ")?;
            attributes.write_as_list(s)?;
            writeln!(s)?;
        }
        Ok(())
    }
}

impl<I, K, V> From<I> for Attributes
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<Id>,
    V: Into<Id>,
{
    fn from(value: I) -> Self {
        Self {
            attributes: value
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }
}
impl Attributes {
    pub fn add(&mut self, key: impl Into<Id>, value: impl Into<Id>) {
        self.attributes.insert(key.into(), value.into());
    }
    fn write_as_list<S: Write>(&self, s: &mut S) -> Result<()> {
        write!(s, "[")?;
        let mut iter = self.attributes.iter().peekable();
        while let Some((key, value)) = iter.next() {
            key.write(s)?;
            write!(s, " = ")?;
            value.write(s)?;
            if iter.peek().is_some() {
                write!(s, "; ")?;
            }
        }
        write!(s, "]")?;
        Ok(())
    }
    fn write_as_statements<S: Write>(&self, s: &mut S) -> Result<()> {
        for (key, value) in &self.attributes {
            key.write(s)?;
            write!(s, " = ")?;
            value.write(s)?;
            writeln!(s, ";")?;
        }
        Ok(())
    }
    fn write_append_list<S: Write>(&self, s: &mut S) -> Result<()> {
        if !self.attributes.is_empty() {
            write!(s, " ")?;
            self.write_as_list(s)?;
        }
        writeln!(s)?;
        Ok(())
    }
}
impl Node {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            attributes: Default::default(),
        }
    }
    pub fn new_attr(id: impl Into<String>, attributes: impl Into<Attributes>) -> Self {
        Self {
            id: id.into(),
            attributes: attributes.into(),
        }
    }
    fn write<S: Write>(&self, s: &mut S) -> Result<()> {
        write!(s, "{}", escape_id(&self.id))?;
        self.attributes.write_append_list(s)?;
        Ok(())
    }
}
impl Edge {
    pub fn new(a: impl Into<String>, b: impl Into<String>) -> Self {
        Self {
            a: a.into(),
            a_compass: None,
            b: b.into(),
            b_compass: None,
            attributes: Default::default(),
        }
    }
    pub fn new_attr(
        a: impl Into<String>,
        b: impl Into<String>,
        attributes: impl Into<Attributes>,
    ) -> Self {
        Self {
            a: a.into(),
            a_compass: None,
            b: b.into(),
            b_compass: None,
            attributes: attributes.into(),
        }
    }
    pub fn new_compass(
        a: impl Into<String>,
        a_compass: Option<impl Into<String>>,
        b: impl Into<String>,
        b_compass: Option<impl Into<String>>,
        attributes: impl Into<Attributes>,
    ) -> Self {
        Self {
            a: a.into(),
            a_compass: a_compass.map(Into::into),
            b: b.into(),
            b_compass: b_compass.map(Into::into),
            attributes: attributes.into(),
        }
    }
    fn write<S: Write>(&self, s: &mut S) -> Result<()> {
        write!(s, "{}", escape_id(&self.a))?;
        if let Some(compass) = &self.a_compass {
            write!(s, ":{compass}")?;
        }
        write!(s, " -> {}", escape_id(&self.b))?;
        if let Some(compass) = &self.b_compass {
            write!(s, ":{compass}")?;
        }

        self.attributes.write_append_list(s)?;
        Ok(())
    }
}
impl Subgraph {
    fn write<S: Write>(&self, s: &mut S) -> Result<()> {
        if let Some(id) = &self.id {
            write!(s, "{}", escape_id(id))?;
        }
        writeln!(s, "{{")?;
        self.base.write(s)?;
        writeln!(s, "}}")?;
        Ok(())
    }
}

fn escape_id(id: &str) -> String {
    format!(r#""{}""#, id.replace("\"", "\\\""))
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum XmlNode {
    Text(String),
    Tag(XmlTag),
}
impl<S: Into<String>> From<S> for XmlNode {
    fn from(value: S) -> Self {
        Self::Text(value.into())
    }
}
impl From<XmlTag> for XmlNode {
    fn from(value: XmlTag) -> Self {
        Self::Tag(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct XmlTag {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub body: Vec<XmlNode>,
}

impl XmlNode {
    fn write<S: Write>(&self, s: &mut S) -> Result<()> {
        match self {
            XmlNode::Text(text) => write!(
                s,
                "{}",
                text.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('"', "&quot;")
            ),
            XmlNode::Tag(tag) => tag.write(s),
        }
    }
}
impl XmlTag {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            attributes: vec![],
            body: vec![],
        }
    }
    pub fn attr(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.push((key.into(), value.into()));
        self
    }
    pub fn body(mut self, body: impl IntoIterator<Item = impl Into<XmlNode>>) -> Self {
        self.body.extend(body.into_iter().map(Into::into));
        self
    }
    pub fn child(mut self, child: impl Into<XmlNode>) -> Self {
        self.body.push(child.into());
        self
    }
    fn write<S: Write>(&self, s: &mut S) -> Result<()> {
        write!(s, "<{}", self.name)?; // TODO escape
        for (key, value) in &self.attributes {
            write!(s, r#" {key}="{value}""#)?; // TODO escape
        }
        if self.body.is_empty() {
            write!(s, "/>")?;
        } else {
            write!(s, ">")?;
            for item in &self.body {
                item.write(s)?;
            }
            write!(s, "</{}>", self.name)?; // TODO escape
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_dot() {
        let mut graph = Graph::new("digraph");

        graph.base.attributes.add("test", "idk");
        graph.base.attributes.add("another", "one");
        graph.base.graph_attributes.add("test", "graph_value");
        graph.base.graph_attributes.add("another", "one");
        graph.base.node_attributes.add("shape", "plaintext");
        graph.base.node_attributes.add("fontname", "monospace");

        let xml = XmlTag::new("TABLE")
            .attr("BORDER", "0")
            .attr("CELLBORDER", "1")
            .attr("CELLSPACING", "0")
            .body([
                XmlTag::new("TR").body([XmlTag::new("TD")
                    // .attr("ROWSPAN", "3")
                    .attr("BGCOLOR", "yellow")
                    .attr("ALIGN", "left")
                    .body(["class"])]),
                XmlTag::new("TR").body([XmlTag::new("TD")
                    .attr("PORT", "here")
                    .attr("BGCOLOR", "lightblue")
                    .attr("ALIGN", "left")
                    .body(["  buh"])]),
                XmlTag::new("TR").body([XmlTag::new("TD")
                    .attr("PORT", "here")
                    .attr("BGCOLOR", "lightblue")
                    .attr("ALIGN", "left")
                    .body(["  buh??????\n?????"])]),
                XmlTag::new("TR").body([XmlTag::new("TD")
                    .attr("PORT", "here")
                    .attr("BGCOLOR", "lightblue")
                    .attr("ALIGN", "left")
                    .attr("BALIGN", "left")
                    .child("    buh??????")
                    .child(XmlTag::new("BR"))
                    .child("  ??????")
                    .child(XmlTag::new("BR"))
                    .child("????????")]),
            ]);

        let xml2 = XmlTag::new("TABLE")
            .attr("BORDER", "0")
            .attr("CELLBORDER", "1")
            .attr("CELLSPACING", "0")
            .body([
                XmlTag::new("TR")
                    .child(XmlTag::new("TD").child("123"))
                    .child(
                        XmlTag::new("TD")
                            .attr("BGCOLOR", "yellow")
                            .attr("ALIGN", "left")
                            .body(["ExJumpIfNot"]),
                    ),
                XmlTag::new("TR")
                    .child(XmlTag::new("TD").child("condition"))
                    .child(
                        XmlTag::new("TD")
                            .attr("CELLPADDING", "0")
                            .attr("BORDER", "0")
                            .child(
                                XmlTag::new("TABLE")
                                    .attr("BORDER", "0")
                                    .attr("CELLBORDER", "1")
                                    .attr("CELLSPACING", "0")
                                    .body([
                                        XmlTag::new("TR")
                                            .child(XmlTag::new("TD").child("123"))
                                            .child(
                                                XmlTag::new("TD")
                                                    .attr("BGCOLOR", "yellow")
                                                    .attr("ALIGN", "left")
                                                    .body(["ExJumpIfNot"]),
                                            ),
                                        XmlTag::new("TR")
                                            .child(XmlTag::new("TD").child("condition"))
                                            .child(XmlTag::new("TD").child("some variable")),
                                    ]),
                            ),
                    ),
            ]);

        graph.base.nodes.push(Node::new_attr(
            "buh node b",
            [("label", Id::Html(xml2.into()))],
        ));

        graph.base.nodes.push(Node::new_attr(
            "buh node a",
            [("label", Id::Html(xml.into()))],
        ));

        graph.base.edges.push(Edge::new_attr(
            "buh node a",
            "buh node b",
            [("something", "other"), ("asdfasfsa", "guh")],
        ));

        let mut subgraph = Subgraph::default();
        subgraph.base.attributes.add("rank", "min");
        subgraph.base.nodes.push(Node::new("buh node a"));
        subgraph.base.nodes.push(Node::new("buh node b"));

        graph.base.subgraphs.push(subgraph);

        dbg!(&graph);

        let mut s = String::new();
        graph.write(&mut s).unwrap();

        println!("{s}");
    }
}

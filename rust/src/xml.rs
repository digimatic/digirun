//! A small namespace-agnostic XML tree.
//!
//! The Python version walks `ElementTree` and matches elements by their local
//! name, which is what these files need: GPX and TCX use different namespace
//! prefixes for the same elements (`ns3:TrackPointExtension` vs `TPX`).

use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, Default)]
pub struct Node {
    /// Local name of the element, without its namespace prefix.
    pub name: String,
    pub attrs: Vec<(String, String)>,
    /// Text directly inside the element.
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    /// First child element with this local name.
    pub fn find(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Trimmed text of the first child with this local name, or `""`.
    pub fn text(&self, name: &str) -> String {
        match self.find(name) {
            Some(child) => child.text.trim().to_string(),
            None => String::new(),
        }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Children with this local name, in document order.
    pub fn children_named(&self, name: &str) -> Vec<&Node> {
        self.children.iter().filter(|c| c.name == name).collect()
    }
}

/// Parse a document into its root element.
pub fn parse(xml: &str) -> Result<Node, String> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<Node> = Vec::new();
    let mut root = Node::default();

    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => stack.push(node(&e)),
            Ok(Event::Empty(e)) => {
                let node = node(&e);
                attach(&mut stack, &mut root, node);
            }
            Ok(Event::Text(t)) => {
                // Only text directly inside an element counts, as in ElementTree.
                let value = match t.unescape() {
                    Ok(value) => value.into_owned(),
                    Err(_) => String::from_utf8_lossy(t.as_ref()).into_owned(),
                };
                match stack.last_mut() {
                    Some(top) => top.text.push_str(&value),
                    None => root.text.push_str(&value),
                }
            }
            Ok(Event::End(_)) => {
                if let Some(node) = stack.pop() {
                    attach(&mut stack, &mut root, node);
                }
            }
            Ok(_) => {}
            Err(e) => return Err(e.to_string()),
        }
    }

    if root.name.is_empty() {
        return Err("no root element".to_string());
    }
    Ok(root)
}

fn node(start: &quick_xml::events::BytesStart) -> Node {
    Node {
        name: local(start.name()),
        attrs: start
            .attributes()
            .filter_map(|a| a.ok())
            .map(|a| {
                (
                    local(a.key),
                    a.unescape_value().unwrap_or_default().into_owned(),
                )
            })
            .collect(),
        text: String::new(),
        children: Vec::new(),
    }
}

fn attach(stack: &mut Vec<Node>, root: &mut Node, node: Node) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(node),
        None => *root = node,
    }
}

fn local(name: quick_xml::name::QName) -> String {
    String::from_utf8_lossy(name.local_name().as_ref()).into_owned()
}

//! Locator selector strategies.

use serde_json::{json, Value};

/// The ARIA role used by semantic locators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// A button.
    Button,
    /// A hyperlink.
    Link,
    /// A single-line text input.
    TextBox,
    /// A search input.
    SearchBox,
    /// A checkbox.
    CheckBox,
    /// A radio button.
    Radio,
    /// A heading.
    Heading,
    /// An image.
    Img,
    /// A list container.
    List,
    /// A list item.
    ListItem,
    /// A combobox / select.
    ComboBox,
    /// A listbox / multi-select.
    ListBox,
    /// A navigation landmark.
    Navigation,
    /// The main landmark.
    Main,
    /// A banner landmark.
    Banner,
    /// A content-info landmark.
    ContentInfo,
    /// A form landmark.
    Form,
    /// A table.
    Table,
    /// A table row.
    Row,
    /// A table cell.
    Cell,
    /// A dialog.
    Dialog,
    /// A generic region.
    Region,
}

impl Role {
    /// The canonical ARIA role string.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Button => "button",
            Role::Link => "link",
            Role::TextBox => "textbox",
            Role::SearchBox => "searchbox",
            Role::CheckBox => "checkbox",
            Role::Radio => "radio",
            Role::Heading => "heading",
            Role::Img => "img",
            Role::List => "list",
            Role::ListItem => "listitem",
            Role::ComboBox => "combobox",
            Role::ListBox => "listbox",
            Role::Navigation => "navigation",
            Role::Main => "main",
            Role::Banner => "banner",
            Role::ContentInfo => "contentinfo",
            Role::Form => "form",
            Role::Table => "table",
            Role::Row => "row",
            Role::Cell => "cell",
            Role::Dialog => "dialog",
            Role::Region => "region",
        }
    }
}

/// A strategy for finding elements on a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    /// A CSS selector.
    Css(String),
    /// Elements whose rendered text matches.
    Text {
        /// The text to match.
        text: String,
        /// Whether the match must be exact.
        exact: bool,
    },
    /// Elements with a given ARIA role and, optionally, an accessible name.
    Role {
        /// The role to match.
        role: Role,
        /// The accessible name to match.
        name: Option<String>,
    },
    /// Elements with a matching `placeholder` attribute.
    Placeholder {
        /// The placeholder text to match.
        text: String,
        /// Whether the match must be exact.
        exact: bool,
    },
    /// Form controls associated with a matching `<label>`.
    Label {
        /// The label text to match.
        text: String,
        /// Whether the match must be exact.
        exact: bool,
    },
    /// Images with matching `alt` text.
    AltText {
        /// The alt text to match.
        text: String,
        /// Whether the match must be exact.
        exact: bool,
    },
    /// Elements with a matching test id (`data-testid` and friends).
    TestId(String),
}

impl Selector {
    /// Create a CSS selector.
    pub fn css(selector: impl Into<String>) -> Self {
        Selector::Css(selector.into())
    }

    /// Create a text selector.
    pub fn text(text: impl Into<String>, exact: bool) -> Self {
        Selector::Text {
            text: text.into(),
            exact,
        }
    }

    /// Create a role selector.
    pub fn role(role: Role, name: Option<impl Into<String>>) -> Self {
        Selector::Role {
            role,
            name: name.map(Into::into),
        }
    }

    /// Create a placeholder selector.
    pub fn placeholder(text: impl Into<String>, exact: bool) -> Self {
        Selector::Placeholder {
            text: text.into(),
            exact,
        }
    }

    /// Create a label selector.
    pub fn label(text: impl Into<String>, exact: bool) -> Self {
        Selector::Label {
            text: text.into(),
            exact,
        }
    }

    /// Create an alt-text selector.
    pub fn alt_text(text: impl Into<String>, exact: bool) -> Self {
        Selector::AltText {
            text: text.into(),
            exact,
        }
    }

    /// Create a test-id selector.
    pub fn test_id(id: impl Into<String>) -> Self {
        Selector::TestId(id.into())
    }

    /// A human-readable description for diagnostics and error messages.
    pub fn describe(&self) -> String {
        match self {
            Selector::Css(value) => format!("css={value}"),
            Selector::Text { text, exact } => {
                if *exact {
                    format!("text={text:?} (exact)")
                } else {
                    format!("text={text:?}")
                }
            }
            Selector::Role { role, name } => match name {
                Some(name) => format!("role={} name={name:?}", role.as_str()),
                None => format!("role={}", role.as_str()),
            },
            Selector::Placeholder { text, exact } => {
                format!("placeholder={text:?}{}", if *exact { " exact" } else { "" })
            }
            Selector::Label { text, exact } => {
                format!("label={text:?}{}", if *exact { " exact" } else { "" })
            }
            Selector::AltText { text, exact } => {
                format!("alt={text:?}{}", if *exact { " exact" } else { "" })
            }
            Selector::TestId(id) => format!("testid={id:?}"),
        }
    }

    /// Serialize to the JSON spec consumed by the injected page script.
    pub fn to_spec(&self) -> Value {
        match self {
            Selector::Css(value) => json!({ "kind": "css", "value": value }),
            Selector::Text { text, exact } => {
                json!({ "kind": "text", "value": text, "exact": exact })
            }
            Selector::Role { role, name } => json!({
                "kind": "role",
                "role": role.as_str(),
                "name": name,
                "exact": false,
            }),
            Selector::Placeholder { text, exact } => {
                json!({ "kind": "placeholder", "value": text, "exact": exact })
            }
            Selector::Label { text, exact } => {
                json!({ "kind": "label", "value": text, "exact": exact })
            }
            Selector::AltText { text, exact } => {
                json!({ "kind": "alt", "value": text, "exact": exact })
            }
            Selector::TestId(id) => json!({ "kind": "testid", "value": id }),
        }
    }
}

impl From<&str> for Selector {
    fn from(value: &str) -> Self {
        Selector::Css(value.to_string())
    }
}

impl From<String> for Selector {
    fn from(value: String) -> Self {
        Selector::Css(value)
    }
}

impl From<&String> for Selector {
    fn from(value: &String) -> Self {
        Selector::Css(value.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_css_specs() {
        assert_eq!(
            Selector::css("input").to_spec(),
            json!({ "kind": "css", "value": "input" })
        );
    }

    #[test]
    fn serializes_text_specs() {
        assert_eq!(
            Selector::text("Login", false).to_spec(),
            json!({ "kind": "text", "value": "Login", "exact": false })
        );
    }

    #[test]
    fn serializes_role_specs() {
        let spec = Selector::role(Role::Button, Some("Submit")).to_spec();
        assert_eq!(spec["kind"], "role");
        assert_eq!(spec["role"], "button");
        assert_eq!(spec["name"], "Submit");
    }

    #[test]
    fn describes_selectors_for_errors() {
        assert_eq!(Selector::css("input").describe(), "css=input");
        assert_eq!(
            Selector::role(Role::Button, Some("Go")).describe(),
            "role=button name=\"Go\""
        );
    }
}

//! Graph data shapes. Both `Node` and `Edge` carry user-provided string
//! IDs in v0.0.1: there is no engine-assigned `_id`. Schema (label set,
//! property typing) is the controller's job — Motif treats labels and
//! property keys as opaque strings.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::value::Value;

pub type Properties = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub properties: Properties,
}

impl Node {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            properties: Properties::new(),
        }
    }

    pub fn with_property(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.properties.insert(key.into(), value.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub label: String,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub properties: Properties,
}

impl Edge {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            from: from.into(),
            to: to.into(),
            properties: Properties::new(),
        }
    }

    pub fn with_property(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.properties.insert(key.into(), value.into());
        self
    }
}

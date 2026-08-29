//! Command snippets for the context panel.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snippet {
    pub id: Uuid,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnippetsFile {
    pub version: u32,
    pub snippets: Vec<Snippet>,
}

impl Default for SnippetsFile {
    fn default() -> Self {
        Self {
            version: 1,
            snippets: Vec::new(),
        }
    }
}

impl SnippetsFile {
    pub fn add(&mut self, title: impl Into<String>, body: impl Into<String>) -> Uuid {
        let id = Uuid::new_v4();
        self.snippets.push(Snippet {
            id,
            title: title.into(),
            body: body.into(),
        });
        id
    }

    pub fn remove(&mut self, id: Uuid) -> bool {
        let before = self.snippets.len();
        self.snippets.retain(|s| s.id != id);
        self.snippets.len() != before
    }

    pub fn update(&mut self, id: Uuid, title: String, body: String) -> bool {
        if let Some(s) = self.snippets.iter_mut().find(|s| s.id == id) {
            s.title = title;
            s.body = body;
            true
        } else {
            false
        }
    }
}

//! Dialogue tree component for NPCs and interactive entities.

use std::collections::HashMap;

/// Unique identifier for a node inside a [DialogueTree].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub u32);

/// A single player-selectable response inside a dialogue node.
#[derive(Debug, Clone)]
pub struct DialogueChoice {
    /// Text shown to the player.
    pub text: String,
    /// Node to jump to when this choice is picked.  None = end conversation.
    pub next: Option<NodeId>,
}

impl DialogueChoice {
    pub fn new(text: impl Into<String>, next: Option<NodeId>) -> Self {
        Self {
            text: text.into(),
            next,
        }
    }

    /// Shorthand: choice that leads to another node.
    pub fn goto(text: impl Into<String>, next: NodeId) -> Self {
        Self::new(text, Some(next))
    }

    /// Shorthand: choice that ends the conversation.
    pub fn end(text: impl Into<String>) -> Self {
        Self::new(text, None)
    }
}

/// One "page" of dialogue: a speaker line plus zero or more player choices.
#[derive(Debug, Clone)]
pub struct DialogueNode {
    /// Name (or tag) of the speaker, e.g. "Blacksmith".
    pub speaker: String,
    /// The line of dialogue spoken by the NPC.
    pub text: String,
    /// Player responses.  Empty = auto-advance to next.
    pub choices: Vec<DialogueChoice>,
    /// When there are no choices, conversation advances here.
    /// None = conversation ends after this node.
    pub next: Option<NodeId>,
}

impl DialogueNode {
    /// Speech node with explicit choices.
    pub fn with_choices(
        speaker: impl Into<String>,
        text: impl Into<String>,
        choices: Vec<DialogueChoice>,
    ) -> Self {
        Self {
            speaker: speaker.into(),
            text: text.into(),
            choices,
            next: None,
        }
    }

    /// Speech node that auto-advances to the next node (no player choice).
    pub fn say(speaker: impl Into<String>, text: impl Into<String>, next: Option<NodeId>) -> Self {
        Self {
            speaker: speaker.into(),
            text: text.into(),
            choices: Vec::new(),
            next,
        }
    }
}

/// A complete conversation tree.  Attach to an NPC entity to make it talkable.
pub struct DialogueTree {
    nodes: HashMap<NodeId, DialogueNode>,
    pub start: NodeId,
}

impl DialogueTree {
    pub fn new(start: NodeId) -> Self {
        Self {
            nodes: HashMap::new(),
            start,
        }
    }

    /// Insert a node. Returns &mut Self for chaining.
    pub fn add(&mut self, id: NodeId, node: DialogueNode) -> &mut Self {
        self.nodes.insert(id, node);
        self
    }

    /// Look up a node by id.
    pub fn get(&self, id: NodeId) -> Option<&DialogueNode> {
        self.nodes.get(&id)
    }

    /// Total number of nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Marker component: the entity currently has an open dialogue.
///
/// Attach to the *player* entity (or whoever is talking).
/// Game code reads current_node to know which [DialogueNode] to display
/// and calls [dvance] or [choose] when the player acts.
pub struct ActiveDialogue {
    /// Entity that owns the [DialogueTree] we are reading from.
    pub tree_owner: crate::ecs::Entity,
    /// Current position in the tree.
    pub current_node: NodeId,
}

impl ActiveDialogue {
    pub fn start(tree_owner: crate::ecs::Entity, tree: &DialogueTree) -> Self {
        Self {
            tree_owner,
            current_node: tree.start,
        }
    }

    /// Advance through a node that has no choices (auto-advance).
    /// Returns None if the conversation ends.
    pub fn advance(&mut self, tree: &DialogueTree) -> Option<NodeId> {
        if let Some(node) = tree.get(self.current_node) {
            if let Some(next) = node.next {
                self.current_node = next;
                return Some(next);
            }
        }
        None
    }

    /// Pick a choice by index.  Returns None if the conversation ends.
    pub fn choose(&mut self, tree: &DialogueTree, index: usize) -> Option<NodeId> {
        if let Some(node) = tree.get(self.current_node) {
            if let Some(choice) = node.choices.get(index) {
                if let Some(next) = choice.next {
                    self.current_node = next;
                    return Some(next);
                }
            }
        }
        None
    }
}

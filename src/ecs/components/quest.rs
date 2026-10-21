//! Quest / journal ECS components.

/// Unique quest identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QuestId(pub u32);

/// Current lifecycle state of a quest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestState {
    /// The player knows about it but has not started.
    Available,
    /// In progress.
    Active,
    /// All objectives met, ready to turn in.
    Complete,
    /// Turned in / fully finished.
    Finished,
    /// Player declined or it expired.
    Failed,
}

/// A single trackable objective within a quest.
#[derive(Debug, Clone)]
pub struct QuestObjective {
    pub description: String,
    pub current: u32,
    pub required: u32,
}

impl QuestObjective {
    pub fn new(description: impl Into<String>, required: u32) -> Self {
        Self {
            description: description.into(),
            current: 0,
            required,
        }
    }

    /// Is this individual objective satisfied?
    #[inline]
    pub fn done(&self) -> bool {
        self.current >= self.required
    }

    /// Increment progress, clamped at required.  Returns true the
    /// first time the objective reaches completion.
    pub fn advance(&mut self, amount: u32) -> bool {
        let was_done = self.done();
        self.current = (self.current + amount).min(self.required);
        !was_done && self.done()
    }
}

/// A quest definition + live progress.  Attach to a "quest giver" or store
/// centrally in a [QuestLog].
#[derive(Debug, Clone)]
pub struct Quest {
    pub id: QuestId,
    pub name: String,
    pub description: String,
    pub state: QuestState,
    pub objectives: Vec<QuestObjective>,
    /// Optional XP reward.
    pub xp_reward: u32,
}

impl Quest {
    pub fn new(
        id: QuestId,
        name: impl Into<String>,
        description: impl Into<String>,
        objectives: Vec<QuestObjective>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            description: description.into(),
            state: QuestState::Available,
            objectives,
            xp_reward: 0,
        }
    }

    /// Builder helper: set XP reward.
    pub fn with_xp(mut self, xp: u32) -> Self {
        self.xp_reward = xp;
        self
    }

    /// Are all objectives satisfied?
    pub fn objectives_met(&self) -> bool {
        self.objectives.iter().all(|o| o.done())
    }

    /// Advance objective at index.  Automatically transitions to
    /// [QuestState::Complete] when all objectives are met.
    pub fn advance_objective(&mut self, index: usize, amount: u32) {
        if self.state != QuestState::Active {
            return;
        }
        if let Some(obj) = self.objectives.get_mut(index) {
            obj.advance(amount);
        }
        if self.objectives_met() {
            self.state = QuestState::Complete;
        }
    }

    /// Activate the quest (Available -> Active).
    pub fn accept(&mut self) {
        if self.state == QuestState::Available {
            self.state = QuestState::Active;
        }
    }

    /// Turn in a completed quest (Complete -> Finished).  Returns XP reward.
    pub fn turn_in(&mut self) -> u32 {
        if self.state == QuestState::Complete {
            self.state = QuestState::Finished;
            self.xp_reward
        } else {
            0
        }
    }

    /// Fail the quest.
    pub fn fail(&mut self) {
        if self.state == QuestState::Active {
            self.state = QuestState::Failed;
        }
    }
}

/// ECS component: a journal of quests carried by an entity (typically the player).
pub struct QuestLog {
    pub quests: Vec<Quest>,
}

impl QuestLog {
    pub fn new() -> Self {
        Self { quests: Vec::new() }
    }

    /// Add a quest to the log.
    pub fn add(&mut self, quest: Quest) {
        self.quests.push(quest);
    }

    /// Find a quest by id.
    pub fn get(&self, id: QuestId) -> Option<&Quest> {
        self.quests.iter().find(|q| q.id == id)
    }

    /// Find a quest by id (mutable).
    pub fn get_mut(&mut self, id: QuestId) -> Option<&mut Quest> {
        self.quests.iter_mut().find(|q| q.id == id)
    }

    /// All quests in a given state.
    pub fn by_state(&self, state: QuestState) -> Vec<&Quest> {
        self.quests.iter().filter(|q| q.state == state).collect()
    }

    /// Shorthand: active quests.
    pub fn active(&self) -> Vec<&Quest> {
        self.by_state(QuestState::Active)
    }

    /// Shorthand: completed (ready to turn in).
    pub fn completed(&self) -> Vec<&Quest> {
        self.by_state(QuestState::Complete)
    }
}

impl Default for QuestLog {
    fn default() -> Self {
        Self::new()
    }
}

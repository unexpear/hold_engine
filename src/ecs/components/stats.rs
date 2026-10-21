//! RPG-style stats component.

/// Core stats that any entity with health or combat properties can carry.
///
/// All values are plain `f32` so they work with the existing ECS pipeline
/// without additional trait bounds.
pub struct Stats {
    pub health: f32,
    pub health_max: f32,
    pub mana: f32,
    pub mana_max: f32,
    pub attack: f32,
    pub defense: f32,
    pub speed: f32,
    pub level: u32,
    pub xp: u32,
    pub xp_next: u32,
}

impl Stats {
    /// Create a fresh level-1 combatant.
    pub fn new(health: f32, mana: f32, attack: f32, defense: f32, speed: f32) -> Self {
        Self {
            health,
            health_max: health,
            mana,
            mana_max: mana,
            attack,
            defense,
            speed,
            level: 1,
            xp: 0,
            xp_next: 100,
        }
    }

    /// Simple constructor for health-only entities (for example, destructibles).
    pub fn from_health(health: f32) -> Self {
        Self::new(health, 0.0, 0.0, 0.0, 0.0)
    }

    /// Returns whether the entity is alive.
    pub fn alive(&self) -> bool {
        self.health > 0.0
    }

    /// Apply damage (reduced by defense and floored at 0). Returns actual damage dealt.
    pub fn damage(&mut self, raw: f32) -> f32 {
        let reduced = (raw - self.defense).max(0.0);
        let dealt = reduced.min(self.health);
        self.health -= dealt;
        dealt
    }

    /// Heal, capped at max health. Returns actual healing applied.
    pub fn heal(&mut self, amount: f32) -> f32 {
        let room = self.health_max - self.health;
        let healed = amount.min(room).max(0.0);
        self.health += healed;
        healed
    }

    /// Spend mana. Returns true if there was enough.
    pub fn spend_mana(&mut self, cost: f32) -> bool {
        if self.mana >= cost {
            self.mana -= cost;
            true
        } else {
            false
        }
    }

    /// Award XP. Returns true if at least one level-up occurred.
    pub fn award_xp(&mut self, amount: u32) -> bool {
        self.xp = self.xp.saturating_add(amount);

        let mut leveled = false;
        while self.xp >= self.xp_next {
            self.xp -= self.xp_next;
            self.level = self.level.saturating_add(1);
            self.xp_next = self.xp_next.saturating_add(self.xp_next / 2); // 1.5x per level
            leveled = true;
        }

        leveled
    }
}

#[cfg(test)]
mod tests {
    use super::Stats;

    #[test]
    fn award_xp_returns_false_when_not_enough_for_level() {
        let mut stats = Stats::new(100.0, 50.0, 10.0, 5.0, 3.0);

        let leveled = stats.award_xp(99);

        assert!(!leveled);
        assert_eq!(stats.level, 1);
        assert_eq!(stats.xp, 99);
        assert_eq!(stats.xp_next, 100);
    }

    #[test]
    fn award_xp_single_level_with_remainder() {
        let mut stats = Stats::new(100.0, 50.0, 10.0, 5.0, 3.0);

        let leveled = stats.award_xp(150);

        assert!(leveled);
        assert_eq!(stats.level, 2);
        assert_eq!(stats.xp, 50);
        assert_eq!(stats.xp_next, 150);
    }

    #[test]
    fn award_xp_can_level_multiple_times() {
        let mut stats = Stats::new(100.0, 50.0, 10.0, 5.0, 3.0);

        let leveled = stats.award_xp(260);

        assert!(leveled);
        assert_eq!(stats.level, 3);
        assert_eq!(stats.xp, 10);
        assert_eq!(stats.xp_next, 225);
    }
}

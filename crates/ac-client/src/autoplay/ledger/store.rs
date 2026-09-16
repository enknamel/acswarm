use crate::Client;

impl Client {
    /// Where this character's loot ledger lives: beside the profiles,
    /// under the server it plays on. Two worlds share no object ids, so
    /// two worlds get two files.
    pub(crate) fn ledger_path(&self) -> Option<std::path::PathBuf> {
        let name = self.world.stats.name.trim();
        if name.is_empty() {
            return None;
        }
        let dir = self.profiles.dir();
        let beside = dir.parent()?;
        Some(ac_loot::Ledger::path_of(beside, &self.config.host, name))
    }

    /// Read back what this character was carrying things for. Done once,
    /// when it enters the world and its name is known.
    pub(crate) fn load_ledger(&mut self) {
        let Some(path) = self.ledger_path() else {
            return;
        };
        let ledger = ac_loot::Ledger::load(&path);
        if !ledger.is_empty() {
            tracing::info!(
                path = %path.display(),
                items = ledger.len(),
                "loot ledger read back"
            );
        }
        self.autoplay.ledger = ledger;
    }

    /// Write it out when it has changed. Every tick, because the thing
    /// it defends against is a crash.
    pub(crate) fn save_ledger(&mut self) {
        if !self.autoplay.ledger.unsaved() {
            return;
        }
        if let Some(path) = self.ledger_path() {
            self.autoplay.ledger.save_if_changed(&path);
        }
    }
}

use std::time::{Duration, Instant};

use crate::{
    dodge, pathfinder, player, profile, reconnect, route, Client, Config, DEFAULT_JUMP_HEIGHT,
    DEFAULT_SPEED_BOOST,
};

impl Client {
    /// Open the sockets, start the login handshake, and return the session.
    pub fn connect(config: Config, assets: std::rc::Rc<ac_scene::Assets>) -> std::io::Result<Self> {
        let host = config.host.clone();
        // Resolve the login address, accepting a hostname or an IP, with or
        // without a port (the public servers are named hosts, so a bare
        // `parse()` to a SocketAddr rejects them; `to_socket_addrs` runs
        // DNS). Default to the login port 9000 when none is given.
        let target = if host.contains(':') {
            host.clone()
        } else {
            format!("{host}:9000")
        };
        let primary: std::net::SocketAddr = std::net::ToSocketAddrs::to_socket_addrs(&target)
            .map_err(std::io::Error::other)?
            .next()
            .ok_or_else(|| std::io::Error::other(format!("no address found for {host}")))?;
        let socket = std::net::UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;
        tracing::info!("connecting to {primary} as {}", config.account);
        Ok(Self::start(config, assets, Some(socket), primary))
    }

    /// A session with no server: no socket and no address lookup, so
    /// nothing it sends goes anywhere and nothing arrives. Tests build
    /// the situation they ask about by setting its state by hand.
    pub fn offline(assets: std::rc::Rc<ac_scene::Assets>) -> Self {
        let config = Config {
            host: "127.0.0.1:1".into(),
            account: "acreborn".into(),
            password: "x".into(),
            character: None,
            auto_enter: true,
        };
        let nowhere = std::net::SocketAddr::from(([127, 0, 0, 1], 1));
        Self::start(config, assets, None, nowhere)
    }

    /// The one place a session is put together, with or without a
    /// server: the login handshake queued, and every default that is not
    /// zero (see the test `an_offline_session_starts_with_the_defaults_of_a_connected_one`).
    fn start(
        config: Config,
        assets: std::rc::Rc<ac_scene::Assets>,
        socket: Option<std::net::UdpSocket>,
        primary: std::net::SocketAddr,
    ) -> Self {
        use ac_net::messages::DatIteration;
        use ac_net::session::{Config as NetConfig, Session};
        let secondary = std::net::SocketAddr::new(primary.ip(), primary.port() + 1);
        let now = Instant::now();
        let mut session = Session::new(
            NetConfig {
                account: config.account.clone(),
                password: config.password.clone(),
                dats: vec![
                    DatIteration {
                        dat_file_id: 1,
                        dat_file_type: 0,
                        iterations: 2072,
                    },
                    DatIteration {
                        dat_file_id: 2,
                        dat_file_type: 0,
                        iterations: 982,
                    },
                ],
                echo_interval: Duration::from_secs(5),
                ack_interval: Duration::from_secs(2),
            },
            now,
        );
        session.login(now);
        let pathfinder = pathfinder::Pathfinder::new(&assets);
        Client {
            profiles: profile::Library::shared(),
            config,
            socket,
            primary,
            secondary,
            session,
            world: ac_world::World::default(),
            assets,
            characters: Vec::new(),
            characters_known: false,
            ddd_done: false,
            enter_requested: false,
            entering: None,
            pending_create: None,
            create_if_missing: None,
            create_attempted: false,
            create_error: None,
            scene_block: None,
            move_to: None,
            move_to_since: Instant::now(),
            move_to_answered: false,
            last_used: None,
            held_run: false,
            move_refused: std::collections::HashMap::new(),
            use_done: None,
            told: None,
            steering: route::Steering::new(Instant::now()),
            clutter: Default::default(),
            pathfinder,
            travel: Default::default(),
            visits: Default::default(),
            combat: false,
            magic: false,
            missile: false,
            attack_height: 2,
            attack_power: 0.5,
            known_spells: Default::default(),
            autoplay: Default::default(),
            ledger_loaded: false,
            log_off_sent: None,
            logged_off: false,
            require_components: false,
            attack_target: None,
            attack_pending: false,
            attacked: None,
            last_attack: Instant::now(),
            attack_backoff: Duration::from_millis(300),
            wants_the_hands: false,
            last_target_name: String::new(),
            sound_tables: Default::default(),
            waves: Default::default(),
            loot_queue: Default::default(),
            loot_inflight: None,
            loot_sent: None,
            loot_merge: None,
            loot_retry: None,
            packs_said_full: Default::default(),
            fellow_updates: false,
            appraise_queue: Default::default(),
            appraise_inflight: Vec::new(),
            pending_store: None,
            selected: None,
            previous_selected: None,
            salvage_open: false,
            pending_jump: None,
            follow: None,
            dodge_to: None,
            dodge: dodge::State::default(),
            speed_boost: DEFAULT_SPEED_BOOST,
            jump_height: DEFAULT_JUMP_HEIGHT,
            movement_rules: player::MovementRules::default(),
            appraisals: std::collections::HashMap::new(),
            last_appraisal: None,
            appraisal_seq: 0,
            book: None,
            book_seq: 0,
            allegiance_room: 0,
            turbine_context: 1,
            last_click: None,
            player: None,
            player_setup: 0,
            quitting: false,
            ended: None,
            last_refusal: None,
            events: Vec::new(),
        }
    }

    /// Send a clean disconnect (flushing it immediately). This marks the
    /// session as ended on purpose: it is never reconnected.
    pub fn disconnect(&mut self, now: Instant) {
        self.quitting = true;
        self.session.disconnect(now);
        self.flush_outgoing();
    }

    /// Ask the server to log the character off: the game's own logout,
    /// with its save and its animation, rather than the connection simply
    /// going away. Nothing is sent from character select, or twice.
    /// Autoplay stops, so nothing is set in motion during the logout.
    ///
    /// Disconnect afterwards, once [`Client::logged_off`] says so (see
    /// `crate::logoff`): the server acts on a disconnect the moment the
    /// packet arrives and on this only when its world thread gets to it,
    /// so the two sent together can see the logout skipped.
    pub fn log_off(&mut self, now: Instant) {
        if self.log_off_sent.is_some()
            || self.world.player_guid.is_none()
            || self.ending().is_some()
        {
            return;
        }
        self.quitting = true;
        self.autoplay.config.enabled = false;
        self.session
            .send_message(ac_net::messages::queue::UI, ac_net::messages::log_off());
        self.log_off_sent = Some(now);
        self.flush_outgoing();
    }

    /// Nothing left to wait for before disconnecting: the server has
    /// logged the character off, it was never asked to (it was at
    /// character select), or the session has already ended.
    pub fn logged_off(&self) -> bool {
        self.logged_off || self.log_off_sent.is_none() || self.ending().is_some()
    }

    /// Why this session ended, or `None` while it is still alive.
    ///
    /// The net session goes to `Terminated` both loudly (a NetError or a
    /// DISCONNECT packet, which also raise `Event::Terminated`) and
    /// quietly (a CharacterError, which only raises `Event::Refused`),
    /// so the state is what is asked rather than the events.
    pub fn ending(&self) -> Option<reconnect::Ending> {
        if self.session.state() != ac_net::session::State::Terminated {
            return None;
        }
        Some(reconnect::classify(
            self.quitting,
            self.last_refusal,
            self.ended.as_deref(),
        ))
    }
}

#[cfg(test)]
mod tests;
